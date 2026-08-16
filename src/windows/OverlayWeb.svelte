<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';

  // Mirrors overlay.rs's constrain_square exactly (see
  // docs/macos-port/OVERLAY-SPEC.md §5 for the derivation + test table this
  // must match) — the Shift-drag square constraint, anchored at the drag
  // start, side clamped to available room so it never exceeds the captured
  // image bounds.
  function constrainSquare(startX: number, startY: number, curX: number, curY: number, maxW: number, maxH: number): [number, number] {
    const dx = curX - startX;
    const dy = curY - startY;
    const sx = dx < 0 ? -1 : 1;
    const sy = dy < 0 ? -1 : 1;
    const roomX = sx < 0 ? startX : Math.max(0, maxW - 1 - startX);
    const roomY = sy < 0 ? startY : Math.max(0, maxH - 1 - startY);
    const side = Math.max(0, Math.min(Math.abs(dx), Math.abs(dy), roomX, roomY));
    return [startX + sx * side, startY + sy * side];
  }

  // OVERLAY-SPEC.md §1 — per-channel dim factors (out of 255), applied to
  // canvas RGBA (Rust's BGRA-ordered comment there maps R<->factor "R",
  // G<->"G", B<->"B" — same numbers, just read in RGBA order here).
  const DIM = {
    link: { r: 158 / 255, g: 148 / 255, b: 150 / 255 },
    image: { r: 148 / 255, g: 158 / 255, b: 150 / 255 },
  };
  const LINK_COLOR = 'rgb(200, 50, 90)';
  const IMAGE_COLOR = 'rgb(50, 200, 90)';

  let canvasEl: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D;

  let imgWidth = 0;
  let imgHeight = 0;
  let originalCanvas: HTMLCanvasElement;
  let dimmedLinkCanvas: HTMLCanvasElement;
  let dimmedImageCanvas: HTMLCanvasElement;
  let ready = false;

  let modeIsImage = false;
  let keyMap: Record<string, [string, string]> = {};

  let isDrawing = false;
  let shiftDown = false;
  let startX = 0, startY = 0, curX = 0, curY = 0;
  let finished = false; // guard against double-invoke (e.g. Esc right after mouseup)
  let unlistenShow: UnlistenFn | undefined;
  let unlistenMode: UnlistenFn | undefined;

  function buildDimmedCanvas(imageData: ImageData, factors: { r: number; g: number; b: number }): HTMLCanvasElement {
    const out = document.createElement('canvas');
    out.width = imageData.width;
    out.height = imageData.height;
    const octx = out.getContext('2d')!;
    const dst = octx.createImageData(imageData.width, imageData.height);
    const src = imageData.data;
    const d = dst.data;
    for (let i = 0; i < src.length; i += 4) {
      d[i] = src[i] * factors.r;
      d[i + 1] = src[i + 1] * factors.g;
      d[i + 2] = src[i + 2] * factors.b;
      d[i + 3] = src[i + 3];
    }
    octx.putImageData(dst, 0, 0);
    return out;
  }

  function effectiveSelection() {
    let ex = curX, ey = curY;
    if (shiftDown) {
      [ex, ey] = constrainSquare(startX, startY, ex, ey, imgWidth, imgHeight);
    }
    const x = Math.min(startX, ex);
    const y = Math.min(startY, ey);
    return { x, y, w: Math.abs(ex - startX), h: Math.abs(ey - startY) };
  }

  function draw() {
    if (!ready) return;
    ctx.drawImage(modeIsImage ? dimmedImageCanvas : dimmedLinkCanvas, 0, 0);
    if (!isDrawing) return;
    const { x, y, w, h } = effectiveSelection();
    if (w <= 1 || h <= 1) return;

    ctx.drawImage(originalCanvas, x, y, w, h, x, y, w, h);
    const color = modeIsImage ? IMAGE_COLOR : LINK_COLOR;
    ctx.strokeStyle = color;
    ctx.lineWidth = 2;
    ctx.strokeRect(x + 1, y + 1, Math.max(0, w - 2), Math.max(0, h - 2));

    const label = `${w} × ${h}`;
    ctx.font = '26px -apple-system, "Segoe UI", sans-serif'; // 2x CSS px to match physical-pixel canvas backing store
    ctx.textBaseline = 'alphabetic';
    ctx.fillStyle = color;
    const metrics = ctx.measureText(label);
    const labelX = x + w / 2 - metrics.width / 2;
    const labelY = y > 50 ? y - 16 : y + h + 36;
    ctx.fillText(label, labelX, labelY);
  }

  function toImagePixel(clientX: number, clientY: number) {
    const rect = canvasEl.getBoundingClientRect();
    const scaleX = imgWidth / rect.width;
    const scaleY = imgHeight / rect.height;
    return {
      x: Math.round((clientX - rect.left) * scaleX),
      y: Math.round((clientY - rect.top) * scaleY),
    };
  }

  async function finish() {
    if (finished) return;
    finished = true;
    const { x, y, w, h } = effectiveSelection();
    if (w >= 5 && h >= 5) {
      await invoke('overlay_finish', { x, y, width: w, height: h });
    } else {
      await invoke('overlay_cancel');
    }
  }

  async function cancel() {
    if (finished) return;
    finished = true;
    await invoke('overlay_cancel');
  }

  // The overlay window is created ONCE and reused (main.rs pre-warms it
  // hidden at startup, then shows/hides it per capture instead of
  // build/close — see overlay_web.rs) — so this component's `onMount` only
  // runs once, at prewarm time, when there's no capture to load yet. Each
  // actual capture instead fires the "overlay-show" event once fresh data
  // (`PENDING` in overlay_web.rs) is ready, so this is called on EVERY
  // show, not just the first.
  async function loadAndReset() {
    isDrawing = false;
    shiftDown = false;
    finished = false;
    ready = false;

    try {
      const meta = await invoke<{
        width: number;
        height: number;
        modeIsImage: boolean;
        keyMap: Record<string, [string, string]>;
      }>('overlay_get_meta');

      const pixels = await invoke<ArrayBuffer>('overlay_get_pixels');

      imgWidth = meta.width;
      imgHeight = meta.height;
      modeIsImage = meta.modeIsImage;
      keyMap = Object.fromEntries(Object.entries(meta.keyMap).map(([k, v]) => [k.toUpperCase(), v]));

      const imageData = new ImageData(new Uint8ClampedArray(pixels), imgWidth, imgHeight);

      originalCanvas = document.createElement('canvas');
      originalCanvas.width = imgWidth;
      originalCanvas.height = imgHeight;
      originalCanvas.getContext('2d')!.putImageData(imageData, 0, 0);

      dimmedLinkCanvas = buildDimmedCanvas(imageData, DIM.link);
      dimmedImageCanvas = buildDimmedCanvas(imageData, DIM.image);

      canvasEl.width = imgWidth;
      canvasEl.height = imgHeight;
      ctx = canvasEl.getContext('2d')!;
      ready = true;
      draw();

      // Only now is the new frame on the canvas — tell Rust it's safe to
      // reveal the window. It stays hidden until this point specifically so
      // the previous capture's selection rectangle never flashes (the window
      // is reused across captures, so its old frame lingers until this
      // draw()).
      //
      // Signalled directly, NOT from inside requestAnimationFrame: rAF is
      // suspended while a window is hidden, and this window is hidden by
      // design right now — waiting for a frame from a window that cannot
      // paint until it's shown is a deadlock (it made the Rust side fall
      // back to its 400ms "ready never arrived" timer on every capture).
      // draw() already wrote into the canvas's backing store synchronously,
      // so whatever is composited on show() is the new frame.
      invoke('overlay_ready');
    } catch (e) {
      // Expected once at startup: prewarm creates this window before any
      // capture exists, so overlay_get_meta legitimately has nothing yet.
      // Real captures always arrive via the "overlay-show" event below,
      // by which point Rust has already set the pending screenshot.
      console.debug('OverlayWeb: no pending screenshot yet', e);
    }
  }

  function onMouseDown(e: MouseEvent) {
    if (e.button === 1) { cancel(); return; } // middle click cancels
    if (e.button !== 0) return;
    const p = toImagePixel(e.clientX, e.clientY);
    isDrawing = true;
    startX = p.x; startY = p.y; curX = p.x; curY = p.y;
    draw();
  }

  function onMouseMove(e: MouseEvent) {
    if (!isDrawing) return;
    const p = toImagePixel(e.clientX, e.clientY);
    curX = p.x; curY = p.y;
    draw();
  }

  function onMouseUp(e: MouseEvent) {
    if (e.button !== 0 || !isDrawing) return;
    isDrawing = false;
    finish();
  }

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    cancel();
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      cancel();
      return;
    }
    if (e.key === 'Shift') {
      if (!e.repeat && isDrawing) { shiftDown = true; draw(); } else { shiftDown = true; }
      return;
    }
    // Plugin hotkeys: single alphanumeric char, case-insensitive — works
    // whether or not a selection is in progress (OVERLAY-SPEC.md §6).
    if (e.key.length === 1 && finished === false) {
      const entry = keyMap[e.key.toUpperCase()];
      if (entry) {
        finished = true;
        invoke('overlay_plugin_call', { path: entry[0], functionId: entry[1] });
      }
    }
  }

  function onKeyUp(e: KeyboardEvent) {
    if (e.key === 'Shift') {
      shiftDown = false;
      if (isDrawing) draw();
    }
  }

  onMount(async () => {
    window.addEventListener('mousedown', onMouseDown);
    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
    window.addEventListener('contextmenu', onContextMenu);
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('keyup', onKeyUp);

    unlistenShow = await listen('overlay-show', () => { loadAndReset(); });
    unlistenMode = await listen<boolean>('overlay-mode-changed', (event) => {
      modeIsImage = event.payload;
      draw();
    });

    // Covers both: (a) prewarm — no pending screenshot yet, this is a no-op
    // (see loadAndReset's catch), and (b) the rare fallback in
    // show_web_overlay where prewarm hadn't finished yet and a fresh window
    // had to be built on the spot, already carrying real pending data.
    await loadAndReset();
  });

  onDestroy(() => {
    unlistenShow?.();
    unlistenMode?.();
    window.removeEventListener('mousedown', onMouseDown);
    window.removeEventListener('mousemove', onMouseMove);
    window.removeEventListener('mouseup', onMouseUp);
    window.removeEventListener('contextmenu', onContextMenu);
    window.removeEventListener('keydown', onKeyDown);
    window.removeEventListener('keyup', onKeyUp);
  });
</script>

<canvas bind:this={canvasEl} class="overlay-canvas"></canvas>

<style>
  .overlay-canvas {
    position: fixed;
    inset: 0;
    width: 100vw;
    height: 100vh;
    cursor: crosshair;
    display: block;
  }
</style>
