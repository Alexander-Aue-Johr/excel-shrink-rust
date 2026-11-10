// Draw retained images only. Unknown areas stay neutral.
export function drawSheetRegion(ctx, sheet, region, options = {}) {
  const width = options.width || ctx.canvas.width;
  const height = options.height || ctx.canvas.height;
  if (sheet.nativeCaptures) {
    const image = sheet.nativeCaptures[options.variant || 'original'];
    // Native pixels retain their size and origin, including differing extents.
    ctx.clearRect(0, 0, width, height);
    if (image) ctx.drawImage(image, 0, 0);
    return;
  }
  ctx.fillStyle = '#293e48';
  ctx.fillRect(0, 0, width, height);
  const image = sheet.previewImage;
  const source = sheet.previewRegion;
  if (!image || !source) return;
  const left = Math.max(region.x, source.x), top = Math.max(region.y, source.y);
  const right = Math.min(region.x + region.w, source.x + source.w);
  const bottom = Math.min(region.y + region.h, source.y + source.h);
  if (left >= right || top >= bottom) return;
  ctx.drawImage(image,
    (left - source.x) / source.w * image.naturalWidth, (top - source.y) / source.h * image.naturalHeight,
    (right - left) / source.w * image.naturalWidth, (bottom - top) / source.h * image.naturalHeight,
    (left - region.x) / region.w * width, (top - region.y) / region.h * height,
    (right - left) / region.w * width, (bottom - top) / region.h * height);
}
