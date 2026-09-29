/**
 * Voice-glow tuning for Neru. Seven distinct accents from the matcha tea-house palette, centre first and
 * then the pairs outward: matcha, yuzu, sakura, ai-iro indigo, hojicha, seiji celadon, beni vermilion.
 * Hue drift is kept small so each accent stays recognisable, and the envelope is fast so the glow tracks
 * syllables rather than whole phrases. Light mode uses deeper inks and a stronger glow for pale surfaces.
 */
const reactive = {
  sensitivity: 4.6,
  threshold: 0.008,
  attack: 0.07,
  release: 0.32,
  flow: 96,
  hueRange: 8,
  hueDuration: 14,
  processingLevel: 0.75,
}

export function neruVoiceGlow(light: boolean) {
  return light
    ? {
        ...reactive,
        colors: ['#4f8f2f', '#d9a012', '#e0507e', '#3b5fc9', '#b8662a', '#1f9c8c', '#d8452f'],
        bandColors: { core: '#3f7a22', above: '#e0507e', mid: '#1f9c8c', below: '#3b5fc9' },
        strength: 1,
        strokeOpacity: 2.2,
        innerOpacity: 1.4,
        bloomOpacity: 1.05,
        saturation: 1.9,
        brightness: 0.78,
        bandStrength: 2.8,
        scale: 1.1,
        idle: 0.4,
        reach: 2.5,
        spread: 1.25,
        bloomHeight: 1.4,
        glowSize: 1.15,
      }
    : {
        ...reactive,
        colors: ['#9fd46f', '#f2cc4d', '#f28aad', '#7f9cf5', '#e0995a', '#5fd1bd', '#f07a5f'],
        bandColors: { core: '#f4f7e6', above: '#f28aad', mid: '#9fd46f', below: '#7f9cf5' },
        saturation: 1.35,
        bandStrength: 1.6,
        idle: 0.3,
        reach: 1.8,
        spread: 1.35,
      }
}
