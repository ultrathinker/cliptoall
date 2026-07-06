<script lang="ts">
  import { onMount, flushSync } from 'svelte';
  import { readImageBase64, saveImageBase64, saveImageToFile } from '../lib/api';
  import iconPencil from '../assets/icon_pencil.png';
  import iconPencilActive from '../assets/icon_pencil_active.png';
  import iconRect from '../assets/icon_rect.png';
  import iconRectActive from '../assets/icon_rect_active.png';
  import iconArrow from '../assets/icon_arrow.png';
  import iconArrowActive from '../assets/icon_arrow_active.png';
  import iconClear from '../assets/icon_clear.png';
  import iconUndo from '../assets/icon_undo.png';

  let { imagePath = '', imageScale = 1, outputScale = 1, onSave, onCancel }: { imagePath: string; imageScale?: number; outputScale?: number; onSave?: (path: string) => void; onCancel?: () => void } = $props();

  let canvas: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D;
  let tempCanvas: HTMLCanvasElement;
  let tempCtx: CanvasRenderingContext2D;
  let tool: 'pencil' | 'rect' | 'arrow' | 'text' = $state('pencil');
  let color = $state('#FF0000');
  let brushSize: 'small' | 'medium' | 'large' = $state('small');
  let undoStack: ImageData[] = [];
  let isDrawing = false;
  let startX = 0;
  let startY = 0;
  let lastX = 0;
  let lastY = 0;
  let originalImage: HTMLImageElement;
  let textInput = $state('');
  let showTextInput = $state(false);
  let textX = $state(0);
  let textY = $state(0);
  let textInputEl: HTMLInputElement;
  let saving = $state(false);
  let displayScale = $state(1);

  // Ctrl + mouse-wheel zoom (inspect small text / see the whole image).
  let canvasArea: HTMLDivElement;
  let zoom = $state(1);
  const ZOOM_MIN = 0.1;
  const ZOOM_MAX = 8;
  const ZOOM_STEP = 1.1;

  // Svelte action — called when element is mounted to DOM (guaranteed to work)
  function autoFocusText(node: HTMLInputElement) {
    textInputEl = node;
    node.focus();
    return {
      destroy() {
        textInputEl = undefined as any;
      }
    };
  }

  const colors = [
    { name: 'Red', value: '#FF0000' },
    { name: 'Green', value: '#008000' },
    { name: 'Blue', value: '#1E90FF' },
    { name: 'Black', value: '#000000' },
    { name: 'Pink', value: 'rgb(200, 50, 90)' },
    { name: 'Maroon', value: 'rgb(71, 25, 47)' },
    { name: 'Brown', value: 'rgb(37, 19, 29)' }
  ];

  const brushSizes = {
    pencil: { small: 4, medium: 10, large: 20 },
    rect: { small: 2, medium: 6, large: 15 },
    text: { small: 24, medium: 40, large: 64 }
  };

  function getLineWidth(): number {
    const key = tool === 'pencil' ? 'pencil' : 'rect';
    return brushSizes[key][brushSize];
  }

  /**
   * Set the canvas CSS size. The stored image was already shrunk by `imageScale`
   * at capture (DPI downscale). We multiply back by it so the editor does NOT
   * divide by the DPI a second time — the double compensation made the image
   * render smaller than it really was (BUGS#3). On the capture monitor this
   * yields exactly the on-screen selection size.
   */
  function applyCanvasCssSize() {
    if (!canvas || !tempCanvas) return;
    const dpr = window.devicePixelRatio || 1;
    const cssW = `${(canvas.width * imageScale * zoom) / dpr}px`;
    const cssH = `${(canvas.height * imageScale * zoom) / dpr}px`;
    canvas.style.width = cssW;
    canvas.style.height = cssH;
    tempCanvas.style.width = cssW;
    tempCanvas.style.height = cssH;
  }

  /** Non-passive wheel listener (Svelte onwheel gives no passive guarantee, and
   *  we must preventDefault to stop the WebView from scrolling/zooming). */
  function wheelZoom(node: HTMLElement) {
    const handler = (e: WheelEvent) => handleWheel(e);
    node.addEventListener('wheel', handler, { passive: false });
    return { destroy() { node.removeEventListener('wheel', handler); } };
  }

  function handleWheel(e: WheelEvent) {
    if (!e.ctrlKey) return; // only Ctrl+wheel zooms; plain wheel scrolls normally
    e.preventDefault();
    if (!canvas || !canvasArea) return;

    const factor = e.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP;
    const next = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom * factor));
    if (next === zoom) return;

    // Keep the content point under the cursor fixed while zooming.
    const areaRect = canvasArea.getBoundingClientRect();
    const px = e.clientX - areaRect.left;
    const py = e.clientY - areaRect.top;
    const contentX = canvasArea.scrollLeft + px;
    const contentY = canvasArea.scrollTop + py;
    const ratio = next / zoom;

    zoom = next;
    applyCanvasCssSize(); // writes inline sizes synchronously
    canvasArea.scrollLeft = contentX * ratio - px;
    canvasArea.scrollTop = contentY * ratio - py;
  }

  function resetZoom() {
    zoom = 1;
    applyCanvasCssSize();
  }

  /** Re-apply CSS size when the window moves to a monitor with a different DPI. */
  function watchDpr() {
    const mq = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    mq.addEventListener(
      'change',
      () => {
        applyCanvasCssSize();
        watchDpr(); // re-arm for the new devicePixelRatio
      },
      { once: true },
    );
  }

  onMount(async () => {
    if (!imagePath) return;

    try {
      const base64 = await readImageBase64(imagePath);

      const img = new Image();
      img.onload = () => {
        originalImage = img;
        // Internal canvas resolution = full image pixel dimensions
        canvas.width = img.width;
        canvas.height = img.height;
        tempCanvas.width = img.width;
        tempCanvas.height = img.height;
        ctx = canvas.getContext('2d')!;
        tempCtx = tempCanvas.getContext('2d')!;
        applyCanvasCssSize();
        ctx.drawImage(img, 0, 0);
        saveState();
        watchDpr();
      };
      img.onerror = (e) => {
        console.error('Failed to load editor image', e);
      };
      const mime = imagePath.toLowerCase().endsWith('.png') ? 'image/png' : 'image/jpeg';
      img.src = `data:${mime};base64,${base64}`;
    } catch (e) {
      console.error('Failed to read image for editor:', e);
    }
  });

  function toCanvasCoords(e: MouseEvent): { x: number; y: number } {
    const rect = canvas.getBoundingClientRect();
    return {
      x: (e.clientX - rect.left) * (canvas.width / rect.width),
      y: (e.clientY - rect.top) * (canvas.height / rect.height)
    };
  }

  // Cap undo history by BOTH step count and total memory: a single 4K frame is
  // ~33 MB, so 50 full frames could reach ~1.6 GB. Bound the total instead (BUGS#8).
  const UNDO_MAX_STEPS = 50;
  const UNDO_MAX_BYTES = 512 * 1024 * 1024;
  function saveState() {
    undoStack.push(ctx.getImageData(0, 0, canvas.width, canvas.height));
    let bytes = undoStack.reduce((sum, d) => sum + d.data.length, 0);
    while (undoStack.length > 1 && (undoStack.length > UNDO_MAX_STEPS || bytes > UNDO_MAX_BYTES)) {
      const dropped = undoStack.shift()!;
      bytes -= dropped.data.length;
    }
  }

  function undo() {
    if (undoStack.length > 1) {
      undoStack.pop();
      ctx.putImageData(undoStack[undoStack.length - 1], 0, 0);
      clearTemp();
    }
  }

  function clearAll() {
    if (!originalImage) return;
    // Keep the pre-clear state on the stack so Clear can be undone (BUGS#8).
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(originalImage, 0, 0);
    saveState();
    clearTemp();
  }

  function clearTemp() {
    if (tempCtx) tempCtx.clearRect(0, 0, tempCanvas.width, tempCanvas.height);
  }

  function handleMouseDown(e: MouseEvent) {
    if (e.button !== 0) return;

    // If text input is active, commit it first and don't open new one
    if (showTextInput) {
      commitText();
      return;
    }

    const coords = toCanvasCoords(e);
    startX = coords.x;
    startY = coords.y;
    lastX = startX;
    lastY = startY;
    isDrawing = true;

    if (tool === 'text') {
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      textX = e.clientX - rect.left;
      textY = e.clientY - rect.top;
      displayScale = rect.width / canvas.width;
      // flushSync forces synchronous DOM update so the input exists
      // immediately, while we're still in the mousedown event context
      flushSync(() => { showTextInput = true; });
      textInputEl?.focus();
      isDrawing = false;
      return;
    }

    if (tool === 'pencil') {
      ctx.beginPath();
      ctx.moveTo(startX, startY);
    }
  }

  function handleMouseMove(e: MouseEvent) {
    if (!isDrawing || tool === 'text') return;

    const coords = toCanvasCoords(e);
    const x = coords.x;
    const y = coords.y;

    if (tool === 'pencil') {
      ctx.strokeStyle = color;
      ctx.fillStyle = color;
      ctx.lineWidth = brushSizes.pencil[brushSize];
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      ctx.lineTo(x, y);
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(x, y, brushSizes.pencil[brushSize] / 2, 0, Math.PI * 2);
      ctx.fill();
      ctx.beginPath();
      ctx.moveTo(x, y);
    } else if (tool === 'rect' || tool === 'arrow') {
      tempCtx.clearRect(0, 0, tempCanvas.width, tempCanvas.height);
      if (tool === 'rect') {
        tempCtx.strokeStyle = color;
        tempCtx.lineWidth = brushSizes.rect[brushSize];
        tempCtx.strokeRect(startX, startY, x - startX, y - startY);
      } else {
        drawArrow(tempCtx, startX, startY, x, y);
      }
    }

    lastX = x;
    lastY = y;
  }

  function finishStroke(x: number, y: number) {
    if (!isDrawing || tool === 'text') return;
    isDrawing = false;

    if (tool === 'rect') {
      ctx.strokeStyle = color;
      ctx.lineWidth = brushSizes.rect[brushSize];
      ctx.strokeRect(startX, startY, x - startX, y - startY);
    } else if (tool === 'arrow') {
      drawArrow(ctx, startX, startY, x, y);
    }

    clearTemp();
    saveState();
  }

  function handleMouseUp(e: MouseEvent) {
    if (!isDrawing || tool === 'text') return;
    const coords = toCanvasCoords(e);
    finishStroke(coords.x, coords.y);
  }

  // Mouse released outside the canvas — finalize at the last tracked point so a
  // rubber-band rect/arrow doesn't stay stuck until the next click (3.15).
  function handleWindowMouseUp() {
    if (isDrawing && tool !== 'text') finishStroke(lastX, lastY);
  }

  function drawArrow(context: CanvasRenderingContext2D, x1: number, y1: number, x2: number, y2: number) {
    const lineWidth = brushSizes.rect[brushSize];
    const headlen = Math.max(18, lineWidth * 3);
    const headAngle = Math.PI / 5; // 36 degrees — wider arrowhead
    const angle = Math.atan2(y2 - y1, x2 - x1);

    // Shorten the line so it ends at the base of the arrowhead (not at the tip)
    const shortenBy = headlen * Math.cos(headAngle);
    const lineEndX = x2 - shortenBy * Math.cos(angle);
    const lineEndY = y2 - shortenBy * Math.sin(angle);

    context.strokeStyle = color;
    context.fillStyle = color;
    context.lineWidth = lineWidth;
    context.lineCap = 'round';
    context.lineJoin = 'round';

    // Draw the line (shortened to arrowhead base)
    context.beginPath();
    context.moveTo(x1, y1);
    context.lineTo(lineEndX, lineEndY);
    context.stroke();

    // Draw the arrowhead triangle
    context.beginPath();
    context.moveTo(x2, y2);
    context.lineTo(x2 - headlen * Math.cos(angle - headAngle), y2 - headlen * Math.sin(angle - headAngle));
    context.lineTo(x2 - headlen * Math.cos(angle + headAngle), y2 - headlen * Math.sin(angle + headAngle));
    context.closePath();
    context.fill();
  }

  function commitText() {
    if (!textInput.trim()) {
      showTextInput = false;
      textInput = '';
      return;
    }
    const fontSize = brushSizes.text[brushSize];
    const rect = canvas.getBoundingClientRect();
    const canvasX = textX * (canvas.width / rect.width);
    const canvasY = textY * (canvas.height / rect.height);
    ctx.font = `${brushSize === 'small' ? 'normal' : 'bold'} ${fontSize}px Segoe UI`;
    ctx.fillStyle = color;
    ctx.textBaseline = 'top';
    ctx.fillText(textInput, canvasX, canvasY);
    saveState();
    showTextInput = false;
    textInput = '';
  }

  function handleTextKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitText();
    } else if (e.key === 'Escape') {
      showTextInput = false;
      textInput = '';
    }
  }

  function handleTextBlur(e: FocusEvent) {
    const related = e.relatedTarget as HTMLElement | null;
    if (related?.closest('.toolbar')) {
      // User clicked a toolbar button (size/color change) — refocus text input
      const input = e.target as HTMLInputElement;
      requestAnimationFrame(() => input?.focus());
      return;
    }
    commitText();
  }

  function handleKeyDown(e: KeyboardEvent) {
    // Compare physical key (e.code), not e.key — Ctrl+Z must work on non-Latin
    // keyboard layouts too, where e.key is a localized character (3.15).
    if (e.ctrlKey && e.code === 'KeyZ') {
      e.preventDefault();
      undo();
    } else if (e.ctrlKey && (e.code === 'Digit0' || e.code === 'Numpad0')) {
      e.preventDefault();
      resetZoom();
    }
  }

  async function save() {
    if (saving) return;
    saving = true;
    try {
      const dataUrl = canvas.toDataURL('image/png');
      const base64 = dataUrl.replace(/^data:image\/png;base64,/, '');
      const newPath = await saveImageBase64(base64);
      if (onSave) onSave(newPath);
    } catch (e) {
      console.error('Failed to save edited image:', e);
      alert('Failed to save: ' + e);
    } finally {
      saving = false;
    }
  }

  let savingFile = $state(false);

  async function saveAsFile() {
    if (savingFile) return;
    savingFile = true;
    try {
      const dataUrl = canvas.toDataURL('image/png');
      const base64 = dataUrl.replace(/^data:image\/png;base64,/, '');
      const tempPath = await saveImageBase64(base64);
      const savedPath = await saveImageToFile(tempPath, outputScale);
      if (savedPath) {
        console.log('Saved to:', savedPath);
      }
    } catch (e) {
      console.error('Failed to save file:', e);
      alert('Failed to save: ' + e);
    } finally {
      savingFile = false;
    }
  }

  function cancel() {
    if (onCancel) onCancel();
  }
</script>

<svelte:window onkeydown={handleKeyDown} onmouseup={handleWindowMouseUp} />

<div class="editor-page">
  <div class="toolbar">
    <button
      class="tool-btn text-btn"
      class:active={tool === 'text'}
      onclick={() => tool = 'text'}
      title="Text"
    >
      <span style="font-size: 18px; font-weight: bold;">T</span>
    </button>

    <button
      class="tool-btn"
      class:active={tool === 'pencil'}
      onclick={() => tool = 'pencil'}
      title="Pencil"
    >
      <img src={tool === 'pencil' ? iconPencilActive : iconPencil} alt="Pencil" />
    </button>

    <button
      class="tool-btn"
      class:active={tool === 'rect'}
      onclick={() => tool = 'rect'}
      title="Rectangle"
    >
      <img src={tool === 'rect' ? iconRectActive : iconRect} alt="Rectangle" />
    </button>

    <button
      class="tool-btn"
      class:active={tool === 'arrow'}
      onclick={() => tool = 'arrow'}
      title="Arrow"
    >
      <img src={tool === 'arrow' ? iconArrowActive : iconArrow} alt="Arrow" />
    </button>

    <button class="tool-btn" onclick={clearAll} title="Clear">
      <img src={iconClear} alt="Clear" />
    </button>

    <div class="separator"></div>

    <div class="size-group">
      <button
        class="size-btn"
        class:active={brushSize === 'small'}
        onclick={() => brushSize = 'small'}
        title="Small"
      >
        <div class="size-line" style="height: 2px;"></div>
      </button>
      <button
        class="size-btn"
        class:active={brushSize === 'medium'}
        onclick={() => brushSize = 'medium'}
        title="Medium"
      >
        <div class="size-line" style="height: 5px;"></div>
      </button>
      <button
        class="size-btn"
        class:active={brushSize === 'large'}
        onclick={() => brushSize = 'large'}
        title="Large"
      >
        <div class="size-line" style="height: 10px;"></div>
      </button>
    </div>

    <div class="separator"></div>

    <div class="color-group">
      {#each colors as c}
        <button
          class="color-btn"
          class:active={color === c.value}
          style="background: {c.value};"
          onclick={() => color = c.value}
          title={c.name}
        ></button>
      {/each}
    </div>

    <button class="tool-btn undo-btn" onclick={undo} title="Undo (Ctrl+Z)">
      <img src={iconUndo} alt="Undo" />
    </button>
  </div>

  <div class="canvas-area" bind:this={canvasArea} use:wheelZoom>
    <div class="canvas-wrapper">
      <canvas
        bind:this={canvas}
        onmousedown={handleMouseDown}
        onmousemove={handleMouseMove}
        onmouseup={handleMouseUp}
        onmouseleave={() => { if (isDrawing && tool !== 'rect' && tool !== 'arrow') isDrawing = false; }}
        class:cursor-crosshair={tool !== 'text'}
        class:cursor-text={tool === 'text'}
      ></canvas>
      <canvas
        bind:this={tempCanvas}
        class="temp-canvas"
      ></canvas>
      {#if showTextInput}
        <input
          type="text"
          use:autoFocusText
          bind:value={textInput}
          onkeydown={handleTextKeydown}
          onblur={handleTextBlur}
          class="text-overlay-input"
          style="left: {textX}px; top: {textY}px; width: calc(100% - {textX}px - 4px); font-size: {brushSizes.text[brushSize] * displayScale}px; font-weight: {brushSize === 'small' ? 'normal' : 'bold'}; color: {color};"
        />
      {/if}
    </div>
  </div>

  <div class="bottom-bar">
    <div class="zoom-control">
      <span class="zoom-hint">Ctrl + wheel = zoom</span>
      <span class="zoom-label">{Math.round(zoom * 100)}%</span>
      <button class="btn-secondary zoom-reset" onclick={resetZoom} disabled={zoom === 1} title="Reset zoom (Ctrl+0)">Reset</button>
    </div>
    <button class="btn-save" onclick={save} disabled={saving}>
      {saving ? 'Saving...' : 'Save'}
    </button>
    <button class="btn-secondary" onclick={saveAsFile} disabled={savingFile}>
      {savingFile ? 'Saving...' : 'Save as file'}
    </button>
    <button class="btn-cancel" onclick={cancel}>Cancel</button>
  </div>
</div>

<style>
  .editor-page {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: rgb(240, 240, 240);
  }

  .toolbar {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    padding: 4px 8px;
    background: rgb(225, 225, 225);
    border-bottom: 1px solid rgb(180, 180, 180);
    flex-shrink: 0;
  }

  .separator {
    width: 1px;
    height: 32px;
    background: rgb(180, 180, 180);
    margin: 0 6px;
  }

  .tool-btn {
    width: 47px;
    height: 43px;
    background: rgb(240, 240, 240);
    border: 1px solid rgb(180, 180, 180);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }

  .tool-btn:hover {
    background: rgb(230, 230, 230);
  }

  .tool-btn.active {
    background: rgb(135, 206, 250);
  }

  .tool-btn img {
    width: 32px;
    height: 32px;
    object-fit: contain;
  }

  .text-btn {
    color: rgb(105, 105, 105);
  }

  .text-btn.active {
    color: rgb(0, 0, 0);
  }

  .undo-btn {
    margin-left: 6px;
  }

  .size-group, .color-group {
    display: flex;
    gap: 2px;
  }

  .size-btn {
    width: 32px;
    height: 32px;
    background: rgb(240, 240, 240);
    border: 1px solid rgb(211, 211, 211);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    padding: 4px;
  }

  .size-btn:hover {
    background: rgb(230, 230, 230);
  }

  .size-btn.active {
    background: rgb(135, 206, 250);
  }

  .size-line {
    width: 100%;
    background: black;
    border-radius: 2px;
  }

  .color-btn {
    width: 24px;
    height: 24px;
    border: 2px solid rgb(105, 105, 105);
    cursor: pointer;
    border-radius: 2px;
    padding: 0;
  }

  .color-btn:hover {
    transform: scale(1.1);
  }

  .color-btn.active {
    border-color: white;
    box-shadow: 0 0 0 1px black;
  }

  .canvas-area {
    flex: 1;
    overflow: auto;
    background: rgb(200, 200, 200);
    min-height: 0;
    display: flex;
  }

  /* In a flex container, margin:auto centers the wrapper when it is smaller than
     the viewport, but when zoomed larger the whole image stays scroll-reachable
     — unlike align-items/justify-content:center, which clip the top-left overflow. */
  .canvas-wrapper {
    position: relative;
    display: inline-block;
    margin: auto;
  }

  .canvas-wrapper canvas {
    display: block;
  }

  .temp-canvas {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  .cursor-crosshair {
    cursor: crosshair;
  }

  .cursor-text {
    cursor: text;
  }

  .text-overlay-input {
    position: absolute;
    border: 1px dashed rgba(0, 0, 0, 0.5);
    outline: none;
    background: rgba(255, 255, 255, 0.3);
    font-family: 'Segoe UI', sans-serif;
    z-index: 2;
    padding: 0 2px;
    box-sizing: border-box;
  }

  .bottom-bar {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    background: var(--bg-surface);
    border-top: 1px solid var(--border);
    flex-shrink: 0;
  }

  .zoom-control {
    margin-right: auto; /* keep Save/Cancel on the right */
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .zoom-hint {
    font-size: 11px;
    color: var(--text-dim);
    opacity: 0.7;
  }

  .zoom-label {
    font-size: 12px;
    color: var(--text-main);
    min-width: 40px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .zoom-reset {
    padding: 2px 10px !important;
    font-size: 12px !important;
  }

  .btn-save, .btn-cancel {
    padding: 4px 20px;
    cursor: pointer;
    font-size: 13px;
    font-family: 'Segoe UI', sans-serif;
    border-radius: 4px;
  }

  .btn-save {
    background: var(--accent);
    color: white;
    border: none;
  }

  .btn-save:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-save:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .btn-secondary {
    padding: 4px 20px;
    cursor: pointer;
    font-size: 13px;
    font-family: 'Segoe UI', sans-serif;
    border-radius: 4px;
    background: var(--bg-input);
    color: var(--text-main);
    border: 1px solid var(--border);
  }

  .btn-secondary:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .btn-secondary:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .btn-cancel {
    background: var(--bg-input);
    color: var(--text-main);
    border: 1px solid var(--border);
  }

  .btn-cancel:hover {
    border-color: var(--accent);
  }
</style>
