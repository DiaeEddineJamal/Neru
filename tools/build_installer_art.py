"""Renders Neru's installer artwork: warm parchment and layered paper-cut hills, in the calm,
editorial style of Anthropic's product pages, with Neru's moss mascot and serif wordmark.

Outputs to app/src-tauri/installer/:
  nsis-sidebar.bmp  328x628  NSIS welcome and finish pages (2x of 164x314, scaled to fit the page)
  nsis-header.bmp   300x114  NSIS page header, right-aligned (2x of 150x57)
  wix-banner.bmp    493x58   MSI page banner (WiX draws the page title over the left side)
  wix-dialog.bmp    493x312  MSI welcome and finish pages (WiX draws text right of x=164)
  dmg-background.png / dmg-background@2x.png  660x400 macOS disk image window

Everything is drawn at 4x and downsampled, so edges and type stay clean.
Run from D:\\Neru:  python tools/build_installer_art.py
"""

import math
import random
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parents[1] / "app"
OUT = ROOT / "src-tauri" / "installer"
FONTS = ROOT / "node_modules" / "@fontsource"
MASCOT = Image.open(ROOT / "public" / "neru-mascot-cutout.png").convert("RGBA")

# Paper palette: parchment and oat with moss greens from the app, and one clay accent.
PARCHMENT = (244, 238, 227)
PARCHMENT_DEEP = (236, 228, 213)
OAT = (226, 214, 194)
TAN = (211, 193, 165)
SAGE = (163, 182, 158)
MOSS = (122, 154, 128)
MOSS_DARK = (86, 115, 93)
CLAY = (217, 119, 87)
INK = (31, 30, 27)
INK_SOFT = (92, 88, 80)
MUTED = (134, 127, 116)

SS = 4  # supersampling factor


def font(family: str, name: str, size: int) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(str(FONTS / family / "files" / name), size)


def serif(size: int, italic: bool = False):
    return font("instrument-serif", f"instrument-serif-latin-400-{'italic' if italic else 'normal'}.woff", size)


def sans(size: int, weight: int = 400):
    return font("inter", f"inter-latin-{weight}-normal.woff", size)


def japanese(size: int):
    for name in ("YuGothM.ttc", "YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"):
        path = Path("C:/Windows/Fonts") / name
        if path.exists():
            return ImageFont.truetype(str(path), size)
    return sans(size)


def grain(size, strength: int, seed: int) -> Image.Image:
    """Fine paper grain: per-pixel noise, softened so it reads as fibre, not static."""
    rng = random.Random(seed)
    noise = Image.effect_noise(size, 40).filter(ImageFilter.GaussianBlur(0.6 * SS / 2))
    noise = noise.point(lambda value: 128 + (value - 128) * strength // 40)
    # A few long faint fibres.
    draw = ImageDraw.Draw(noise)
    for _ in range(size[0] * size[1] // (60000 * SS)):
        x, y = rng.uniform(0, size[0]), rng.uniform(0, size[1])
        angle, length = rng.uniform(0, math.pi), rng.uniform(8, 26) * SS
        draw.line((x, y, x + math.cos(angle) * length, y + math.sin(angle) * length), fill=rng.choice((120, 136)), width=max(1, SS // 3))
    return noise


def parchment(size, color=PARCHMENT, seed=1) -> Image.Image:
    """Warm paper with grain and a gentle darkening toward the edges."""
    base = Image.new("RGB", size, color)
    texture = grain(size, 10, seed)
    base = Image.merge("RGB", [ImageChops.add(channel, texture, 1, -128) for channel in base.split()])
    vignette = Image.new("L", size, 0)
    draw = ImageDraw.Draw(vignette)
    w, h = size
    draw.ellipse((-w * 0.25, -h * 0.2, w * 1.25, h * 1.2), fill=255)
    vignette = vignette.filter(ImageFilter.GaussianBlur(min(w, h) * 0.18))
    edge = Image.new("RGB", size, PARCHMENT_DEEP)
    return Image.composite(base, edge, vignette).convert("RGBA")


def ridge(width: int, base_y: float, amplitude: float, seed: int, waves=(1.1, 2.7, 5.3)) -> list:
    """A soft hill line with a slightly torn, hand-cut edge."""
    rng = random.Random(seed)
    phases = [rng.uniform(0, math.tau) for _ in waves]
    points = []
    step = max(1, SS)
    for x in range(-step, width + 2 * step, step):
        t = x / width
        y = base_y
        for index, (wave, phase) in enumerate(zip(waves, phases)):
            y -= amplitude * math.sin(t * math.pi * wave + phase) / (1 + index * 1.6)
        y += rng.uniform(-0.6, 0.6) * SS  # tiny tears along the cut
        points.append((x, y))
    return points


def paper_layer(canvas: Image.Image, top: list, color, shadow=0.26, seed=0, highlight=True):
    """Cuts a paper shape from `top` down to the bottom edge: soft drop shadow, grain, lit edge."""
    w, h = canvas.size
    polygon = top + [(w + 10 * SS, h + 10 * SS), (-10 * SS, h + 10 * SS)]
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).polygon(polygon, fill=255)
    # Shadow cast upward-left light: offset down, blurred.
    shade = Image.new("L", (w, h), 0)
    ImageDraw.Draw(shade).polygon([(x, y + 3.5 * SS) for x, y in top] + polygon[len(top):], fill=int(255 * shadow))
    shade = shade.filter(ImageFilter.GaussianBlur(4.5 * SS))
    canvas.alpha_composite(Image.merge("RGBA", (*[Image.new("L", (w, h), c) for c in (58, 46, 30)], shade)))
    layer = Image.new("RGB", (w, h), color)
    texture = grain((w, h), 8, seed + 17)
    layer = Image.merge("RGB", [ImageChops.add(channel, texture, 1, -128) for channel in layer.split()])
    canvas.paste(layer, (0, 0), mask)
    if highlight:
        edge = Image.new("L", (w, h), 0)
        ImageDraw.Draw(edge).line(top, fill=70, width=max(1, SS))
        edge = ImageChops.multiply(edge, mask)
        canvas.alpha_composite(Image.merge("RGBA", (*[Image.new("L", (w, h), 255)] * 3, edge)))


def sun(canvas: Image.Image, center, radius, color=CLAY):
    """A cut-paper sun disc with its own soft shadow."""
    w, h = canvas.size
    x, y = center
    shade = Image.new("L", (w, h), 0)
    ImageDraw.Draw(shade).ellipse((x - radius, y - radius + 3 * SS, x + radius, y + radius + 3 * SS), fill=60)
    canvas.alpha_composite(Image.merge("RGBA", (*[Image.new("L", (w, h), 40)] * 3, shade.filter(ImageFilter.GaussianBlur(4 * SS)))))
    disc = Image.new("L", (w, h), 0)
    ImageDraw.Draw(disc).ellipse((x - radius, y - radius, x + radius, y + radius), fill=255)
    layer = Image.new("RGB", (w, h), color)
    texture = grain((w, h), 9, 99)
    layer = Image.merge("RGB", [ImageChops.add(channel, texture, 1, -128) for channel in layer.split()])
    canvas.paste(layer, (0, 0), disc)


def mascot(height: int) -> Image.Image:
    """The moss mascot with its eyes (the cutout has none; the app draws them)."""
    box = MASCOT.getbbox()
    cropped = MASCOT.crop(box)
    width = round(cropped.width * height / cropped.height)
    figure = cropped.resize((width * 2, height * 2), Image.LANCZOS)
    draw = ImageDraw.Draw(figure)
    w, h = figure.size
    for cx in (0.43, 0.58):
        rx, ry = w * 0.045, h * 0.085
        draw.ellipse((w * cx - rx, h * 0.5 - ry, w * cx + rx, h * 0.5 + ry), fill=(248, 246, 240, 255))
    return figure.resize((width, height), Image.LANCZOS)


def place_mascot(canvas: Image.Image, top: list, x_center: float, height: int):
    """Sits the mascot on the ridge line `top` at `x_center`."""
    figure = mascot(height)
    ground = min((y for x, y in top if abs(x - x_center) < figure.width * 0.35), default=canvas.height * 0.7)
    canvas.alpha_composite(figure, (int(x_center - figure.width / 2), int(ground - figure.height * 0.9)))


def landscape(canvas: Image.Image, horizon: float, amplitude: float, mascot_at=None, mascot_height=None, sun_at=None, seed=3):
    """Three cut-paper hills (oat, sage, moss) with an optional sun and the mascot in front."""
    w, h = canvas.size
    if sun_at:
        sun(canvas, (sun_at[0] * w, sun_at[1] * h), sun_at[2] * w)
    back = ridge(w, horizon, amplitude * 1.6, seed, (0.8, 1.9, 4.3))
    paper_layer(canvas, back, OAT, 0.16, seed)
    middle = ridge(w, horizon + amplitude * 1.7, amplitude * 1.3, seed + 1, (1.1, 2.6, 5.4))
    paper_layer(canvas, middle, SAGE, 0.22, seed + 1)
    front = ridge(w, horizon + amplitude * 3.2, amplitude * 1.0, seed + 2, (0.6, 1.8, 4.9))
    if mascot_at is not None:
        place_mascot(canvas, front, mascot_at * w, mascot_height)
    paper_layer(canvas, front, MOSS, 0.28, seed + 2)
    ground = ridge(w, horizon + amplitude * 5.4, amplitude * 0.5, seed + 3, (1.4, 3.3, 6.8))
    paper_layer(canvas, ground, MOSS_DARK, 0.3, seed + 3)


def finish(canvas: Image.Image, size) -> Image.Image:
    return canvas.resize(size, Image.LANCZOS).convert("RGB")


def save_bmp(image: Image.Image, name: str):
    OUT.mkdir(parents=True, exist_ok=True)
    image.convert("RGB").save(OUT / name, format="BMP")


def wordmark(draw: ImageDraw.ImageDraw, x: int, y: int, scale: float, tagline: str | None = None):
    """Serif "Neru", 練る, a clay rule and an optional tagline, top-left at (x, y)."""
    s = SS * scale
    draw.text((x, y), "Neru", font=serif(int(46 * s)), fill=INK)
    draw.text((x + int(2 * s), y + int(54 * s)), "練る · to knead, to refine", font=japanese(int(11 * s)), fill=MUTED)
    draw.rounded_rectangle((x + int(2 * s), y + int(78 * s), x + int(30 * s), y + int(80.5 * s)), radius=int(1.5 * s), fill=CLAY)
    if tagline:
        for index, line in enumerate(tagline.split("\n")):
            draw.text((x + int(2 * s), y + int(92 * s) + index * int(17 * s)), line, font=sans(int(11.5 * s)), fill=INK_SOFT)


def nsis_sidebar():
    final = (328, 628)
    size = (final[0] * SS // 2, final[1] * SS // 2)
    canvas = parchment(size, seed=5)
    draw = ImageDraw.Draw(canvas)
    wordmark(draw, 22 * SS, 30 * SS, 1.0, "A calm, local-first\ncoding agent.")
    landscape(canvas, size[1] * 0.58, 12 * SS, mascot_at=0.34, mascot_height=44 * SS, sun_at=(0.72, 0.6, 0.085), seed=11)
    save_bmp(finish(canvas, final), "nsis-sidebar.bmp")


def nsis_header():
    # Right-aligned by MUI_HEADERIMAGE_RIGHT; the page title is drawn by the installer on the left.
    final = (300, 114)
    size = (final[0] * SS // 2, final[1] * SS // 2)
    canvas = parchment(size, seed=7)
    landscape(canvas, size[1] * 0.50, 2.3 * SS, mascot_at=0.7, mascot_height=19 * SS, sun_at=(0.88, 0.52, 0.05), seed=21)
    # Fade the left edge into plain parchment so it meets the header background seamlessly.
    fade = Image.linear_gradient("L").rotate(90, expand=True).resize(size)
    plain = parchment(size, seed=7)
    canvas = Image.composite(canvas, plain, fade.point(lambda value: min(255, max(0, int((value - 90) * 2.4)))))
    save_bmp(finish(canvas, final), "nsis-header.bmp")


def wix_banner():
    # WiX writes the page title in dark text on the left; keep that area plain parchment.
    final = (493, 58)
    size = (final[0] * SS, final[1] * SS)
    canvas = parchment(size, seed=9)
    art = parchment((170 * SS, final[1] * SS), seed=9)
    landscape(art, art.height * 0.42, 3.2 * SS, mascot_at=0.55, mascot_height=17 * SS, sun_at=(0.84, 0.3, 0.06), seed=31)
    mask = Image.linear_gradient("L").rotate(90, expand=True).resize(art.size).point(lambda value: 255 - value)
    canvas.paste(art, (size[0] - art.width, 0), mask)
    ImageDraw.Draw(canvas).line((0, size[1] - SS, size[0], size[1] - SS), fill=TAN, width=SS)
    save_bmp(finish(canvas, final), "wix-banner.bmp")


def wix_dialog():
    # Left 164px carries the art; WiX lays out the welcome text to the right on parchment.
    final = (493, 312)
    size = (final[0] * SS, final[1] * SS)
    canvas = parchment(size, seed=13)
    panel = parchment((164 * SS, size[1]), color=PARCHMENT_DEEP, seed=14)
    draw = ImageDraw.Draw(panel)
    wordmark(draw, 18 * SS, 22 * SS, 0.82, "A calm, local-first\ncoding agent.")
    landscape(panel, panel.height * 0.62, 7 * SS, mascot_at=0.4, mascot_height=30 * SS, sun_at=(0.74, 0.62, 0.075), seed=41)
    canvas.paste(panel, (0, 0))
    ImageDraw.Draw(canvas).line((164 * SS, 0, 164 * SS, size[1]), fill=OAT, width=SS)
    save_bmp(finish(canvas, final), "wix-dialog.bmp")


def dmg_background(scale: int):
    final = (660 * scale, 400 * scale)
    size = (660 * SS, 400 * SS)
    canvas = parchment(size, seed=17)
    draw = ImageDraw.Draw(canvas)
    title = "Drag Neru into Applications"
    title_font = serif(36 * SS)
    tw = draw.textlength(title, font=title_font)
    draw.text(((size[0] - tw) / 2, 38 * SS), title, font=title_font, fill=INK)
    sub = "練る · to knead, to refine through repeated work."
    sub_font = japanese(13 * SS)
    sw = draw.textlength(sub, font=sub_font)
    draw.text(((size[0] - sw) / 2, 88 * SS), sub, font=sub_font, fill=MUTED)
    # Arrow between the app (180, 210) and Applications (480, 210) icon slots, in clay.
    y = 205 * SS
    draw.line((262 * SS, y, 392 * SS, y), fill=CLAY, width=3 * SS)
    draw.polygon([(400 * SS, y), (384 * SS, y - 9 * SS), (384 * SS, y + 9 * SS)], fill=CLAY)
    landscape(canvas, size[1] * 0.80, 9 * SS, sun_at=None, seed=51)
    OUT.mkdir(parents=True, exist_ok=True)
    name = "dmg-background.png" if scale == 1 else f"dmg-background@{scale}x.png"
    finish(canvas, final).save(OUT / name)


if __name__ == "__main__":
    nsis_sidebar()
    nsis_header()
    wix_banner()
    wix_dialog()
    dmg_background(1)
    dmg_background(2)
    print("Installer art written to", OUT)
