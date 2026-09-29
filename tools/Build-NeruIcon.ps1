param(
  [string]$OutputPath = 'D:\Neru\assets\neru-app-icon-aligned.png'
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$sourcePath = 'D:\Neru\assets\neru-mascot-cutout.png'
$source = [System.Drawing.Image]::FromFile($sourcePath)
$canvas = New-Object System.Drawing.Bitmap 1254, 1254, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$graphics = [System.Drawing.Graphics]::FromImage($canvas)
$brush = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml('#f5f4ef'))

try {
  $graphics.Clear([System.Drawing.Color]::Transparent)
  $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
  $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
  $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality

  # The same cutout used by Mascot.tsx, cropped and enlarged for Windows taskbar sizes.
  $crop = New-Object System.Drawing.Rectangle 106, 240, 1054, 876
  $destination = New-Object System.Drawing.Rectangle 16, 32, 1222, 1164
  $graphics.DrawImage($source, $destination, $crop, [System.Drawing.GraphicsUnit]::Pixel)

  # Match the two ellipses and eye color in Mascot.tsx and index.css.
  $scaleX = $destination.Width / $crop.Width
  $scaleY = $destination.Height / $crop.Height
  foreach ($eye in @(@(718, 668, 37, 88), @(881, 656, 37, 83))) {
    $x = $destination.X + ($eye[0] - $crop.X) * $scaleX
    $y = $destination.Y + ($eye[1] - $crop.Y) * $scaleY
    $rx = $eye[2] * $scaleX
    $ry = $eye[3] * $scaleY
    $graphics.FillEllipse($brush, [float]($x - $rx), [float]($y - $ry), [float](2 * $rx), [float](2 * $ry))
  }

  $canvas.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
  Write-Output $OutputPath
}
finally {
  $brush.Dispose()
  $graphics.Dispose()
  $canvas.Dispose()
  $source.Dispose()
}
