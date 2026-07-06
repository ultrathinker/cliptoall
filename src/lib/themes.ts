export interface Theme {
  name: string;
  bgBase: string;
  bgSurface: string;
  bgInput: string;
  accent: string;
  accentHover: string;
  accentDim: string;
  textMain: string;
  textDim: string;
  border: string;
  bgToolbar: string;
  isDark: boolean;
}

export const themes: Record<string, Theme> = {
  classic: {
    name: 'Classic',
    bgBase: 'rgb(240, 240, 240)',
    bgSurface: 'rgb(255, 255, 255)',
    bgInput: 'rgb(255, 255, 255)',
    accent: 'rgb(0, 120, 215)',
    accentHover: 'rgb(0, 102, 204)',
    accentDim: 'rgb(180, 210, 240)',
    textMain: 'rgb(30, 30, 30)',
    textDim: 'rgb(109, 109, 109)',
    border: 'rgb(180, 180, 180)',
    bgToolbar: 'rgb(225, 225, 225)',
    isDark: false
  },
  mac: {
    name: 'Mac',
    bgBase: 'rgb(246, 246, 246)',
    bgSurface: 'rgb(255, 255, 255)',
    bgInput: 'rgb(255, 255, 255)',
    accent: 'rgb(0, 122, 255)',
    accentHover: 'rgb(0, 100, 220)',
    accentDim: 'rgb(198, 222, 255)',
    textMain: 'rgb(29, 29, 31)',
    textDim: 'rgb(142, 142, 147)',
    border: 'rgb(209, 209, 214)',
    bgToolbar: 'rgb(232, 232, 237)',
    isDark: false
  },
  crimson: {
    name: 'Crimson Night',
    bgBase: 'rgb(26, 15, 21)',
    bgSurface: 'rgb(37, 19, 29)',
    bgInput: 'rgb(71, 25, 47)',
    accent: 'rgb(200, 50, 90)',
    accentHover: 'rgb(170, 40, 75)',
    accentDim: 'rgb(84, 22, 47)',
    textMain: 'rgb(230, 225, 232)',
    textDim: 'rgb(170, 158, 175)',
    border: 'rgb(103, 57, 74)',
    bgToolbar: 'rgb(155, 140, 160)',
    isDark: true
  },
  ocean: {
    name: 'Ocean Night',
    bgBase: 'rgb(15, 20, 30)',
    bgSurface: 'rgb(20, 28, 42)',
    bgInput: 'rgb(30, 48, 72)',
    accent: 'rgb(50, 140, 220)',
    accentHover: 'rgb(40, 115, 190)',
    accentDim: 'rgb(25, 62, 100)',
    textMain: 'rgb(225, 232, 240)',
    textDim: 'rgb(148, 163, 184)',
    border: 'rgb(52, 72, 100)',
    bgToolbar: 'rgb(140, 155, 175)',
    isDark: true
  },
  forest: {
    name: 'Forest Night',
    bgBase: 'rgb(16, 24, 16)',
    bgSurface: 'rgb(22, 34, 22)',
    bgInput: 'rgb(32, 56, 36)',
    accent: 'rgb(60, 180, 90)',
    accentHover: 'rgb(45, 150, 72)',
    accentDim: 'rgb(28, 74, 38)',
    textMain: 'rgb(225, 235, 225)',
    textDim: 'rgb(150, 175, 155)',
    border: 'rgb(58, 82, 58)',
    bgToolbar: 'rgb(145, 160, 145)',
    isDark: true
  }
};
