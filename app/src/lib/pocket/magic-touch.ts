/** Uses bundled WASM and downloaded model bytes; inference has no network calls. */
export async function magicTouch(bytes: ArrayBuffer, photo: string, x: number, y: number, accelerator: 'cpu' | 'gpu') {
  if (![x, y].every(n => Number.isFinite(n) && n >= 0 && n <= 1)) throw new Error('Choose a point inside the photo.')
  const { FilesetResolver, InteractiveSegmenter } = await import('@mediapipe/tasks-vision')
  const files = await FilesetResolver.forVisionTasks(`${import.meta.env.BASE_URL}mediapipe`)
  const image = new Image(); image.src = photo; await image.decode()
  const scale = Math.min(1, 1600 / Math.max(image.naturalWidth, image.naturalHeight))
  const canvas = document.createElement('canvas'); canvas.width = Math.round(image.naturalWidth * scale); canvas.height = Math.round(image.naturalHeight * scale)
  const context = canvas.getContext('2d')!
  context.drawImage(image, 0, 0, canvas.width, canvas.height)
  const model = await InteractiveSegmenter.createFromOptions(files, { baseOptions: { modelAssetBuffer: new Uint8Array(bytes), delegate: accelerator === 'gpu' ? 'GPU' : 'CPU' } })
  try {
    // ponytail: one photo per call bounds memory; use a worker if selection blocks the UI noticeably.
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
    model.setImage(canvas)
    // The package declares BrushMode but does not export it at runtime; 1 is POSITIVE.
    const mask = model.segment([{ brushMode: 1, point: [{ x, y }], isCompleted: true }])
    try {
      const values = mask.getAsFloat32Array(), pixels = context.getImageData(0, 0, canvas.width, canvas.height)
      if (mask.width !== canvas.width || mask.height !== canvas.height) throw new Error('Unexpected segmentation dimensions.')
      for (let i = 0; i < values.length; i++) pixels.data[i * 4 + 3] = Math.round(Math.min(1, Math.max(0, values[i])) * 255)
      context.putImageData(pixels, 0, 0)
      return canvas.toDataURL('image/png')
    } finally { mask.close() }
  } finally { model.close() }
}
