"""Renders Neru's installer artwork from the app's design tokens and fonts.

Outputs to app/src-tauri/installer/:
  nsis-header.bmp   150x57   NSIS page header
  nsis-sidebar.bmp  164x314  NSIS welcome and finish pages
  wix-banner.bmp    493x58   MSI page banner (WiX draws the page title over the left side)
  wix-dialog.bmp    493x312  MSI welcome and finish pages (WiX draws text right of x=164)
  dmg-background.png / dmg-background@2x.png  660x400 macOS disk image window

Run from D:\\Neru:  python tools/build_installer_art.py
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parents[1] / "app"
OUT = ROOT / "src-tauri" / "installer"
FONTS = ROOT / "node_modules" / "@fontsource"
MASCOT = Image.open(ROOT / "public" / "neru-mascot-cutout.png").convert("RGBA")

# Design tokens from src/index.css (dark and light themes).
BG = (21, 21, 21)            # --bg
SURFACE_2 = (27, 29, 25)     # --surface-2
TEXT = (242, 240, 233)       # --text
SECONDARY = (179, 179, 170)  # --secondary
MUTED = (140, 144, 136)      # --muted
BORDER = (48, 50, 45)        # --border
SAGE = (142, 162, 145)       # --sage
MOSS_DEEP = (52, 74, 57)     # --moss-deep
LIGHT_BG = (245, 244, 239)   # light --bg
LIGHT_MOSS = (223, 233, 223) # light --moss-deep
LIGHT_SAGE = (100, 128, 106) # light --sage


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


def glow(size, center, radius, color, strength=0.55):
    """A soft radial wash, like the accent glow at the foot of the onboarding rail."""
    # Transparent pixels carry the glow colour, so blurring never darkens light backgrounds.
    layer = Image.new("RGBA", size, color + (0,))
    draw = ImageDraw.Draw(layer)
    x, y = center
    draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=color + (int(255 * strength),))
    return layer.filter(ImageFilter.GaussianBlur(radius * 0.55))


def mascot(height: int) -> Image.Image:
    box = MASCOT.getbbox()
    cropped = MASCOT.crop(box)
    width = round(cropped.width * height / cropped.height)
    figure = cropped.resize((width * 4, height * 4), Image.LANCZOS)
    # The cutout has no eyes; draw them where the app's Mascot places them (--mascot-eye).
    draw = ImageDraw.Draw(figure)
    w, h = figure.size
    for cx in (0.43, 0.58):
        rx, ry = w * 0.045, h * 0.085
        draw.ellipse((w * cx - rx, h * 0.5 - ry, w * cx + rx, h * 0.5 + ry), fill=(245, 244, 239, 255))
    return figure.resize((width, height), Image.LANCZOS)


def dotted(draw, size, color, step=14, alpha=26):
    """A faint dot grid, echoing the thinking orbs."""
    for y in range(step // 2, size[1], step):
        for x in range(step // 2, size[0], step):
            draw.point((x, y), fill=color + (alpha,))


def base(size, color=BG):
    return Image.new("RGBA", size, color + (255,))


def save_bmp(image: Image.Image, name: str):
    OUT.mkdir(parents=True, exist_ok=True)
    image.convert("RGB").save(OUT / name, format="BMP")


def nsis_sidebar():
    size = (164, 314)
    image = base(size)
    image.alpha_composite(glow(size, (82, 330), 120, MOSS_DEEP, 0.9))
    draw = ImageDraw.Draw(image)
    dotted(draw, size, SAGE, 12, 18)
    figure = mascot(64)
    image.alpha_composite(figure, (22, 40))
    draw.text((22, 118), "Neru", font=serif(44), fill=TEXT)
    draw.text((24, 170), "練る", font=japanese(15), fill=MUTED)
    draw.line((22, 204, 60, 204), fill=SAGE, width=2)
    for index, word in enumerate(("Think.", "Build.", "Refine.")):
        draw.text((22, 216 + index * 22), word, font=serif(20, italic=index == 1), fill=SAGE if index == 1 else SECONDARY)
    save_bmp(image, "nsis-sidebar.bmp")


def nsis_header():
    size = (150, 57)
    image = base(size)
    image.alpha_composite(glow(size, (120, 60), 50, MOSS_DEEP, 0.9))
    draw = ImageDraw.Draw(image)
    figure = mascot(30)
    image.alpha_composite(figure, (14, 14))
    draw.text((14 + figure.width + 8, 11), "Neru", font=serif(28), fill=TEXT)
    save_bmp(image, "nsis-header.bmp")


def wix_banner():
    # WiX writes the page title and description in dark text on the left, so keep that area light.
    size = (493, 58)
    image = base(size, LIGHT_BG)
    image.alpha_composite(glow(size, (470, 29), 70, LIGHT_MOSS, 0.9))
    draw = ImageDraw.Draw(image)
    draw.line((0, 57, 493, 57), fill=(221, 220, 213), width=1)
    figure = mascot(34)
    image.alpha_composite(figure, (493 - figure.width - 18, 12))
    save_bmp(image, "wix-banner.bmp")


def wix_dialog():
    # Left 164px carries the art; WiX lays out the welcome text to the right on a light page.
    size = (493, 312)
    image = base(size, LIGHT_BG)
    panel = base((164, 312))
    panel.alpha_composite(glow((164, 312), (82, 330), 120, MOSS_DEEP, 0.9))
    draw = ImageDraw.Draw(panel)
    dotted(draw, (164, 312), SAGE, 12, 18)
    panel.alpha_composite(mascot(62), (22, 38))
    draw.text((22, 114), "Neru", font=serif(44), fill=TEXT)
    draw.text((24, 166), "練る", font=japanese(15), fill=MUTED)
    draw.line((22, 200, 60, 200), fill=SAGE, width=2)
    for index, word in enumerate(("Think.", "Build.", "Refine.")):
        draw.text((22, 212 + index * 22), word, font=serif(20, italic=index == 1), fill=SAGE if index == 1 else SECONDARY)
    image.alpha_composite(panel, (0, 0))
    save_bmp(image, "wix-dialog.bmp")


def dmg_background(scale: int):
    width, height = 660 * scale, 400 * scale
    size = (width, height)
    image = base(size)
    image.alpha_composite(glow(size, (width // 2, height + 40 * scale), 260 * scale, MOSS_DEEP, 0.85))
    draw = ImageDraw.Draw(image)
    dotted(draw, size, SAGE, 16 * scale, 16)
    title = "Drag Neru into Applications"
    title_font = serif(34 * scale)
    tw = draw.textlength(title, font=title_font)
    draw.text(((width - tw) / 2, 44 * scale), title, font=title_font, fill=TEXT)
    sub = "練る · to knead, to refine through repeated work."
    sub_font = sans(13 * scale)
    sw = draw.textlength(sub, font=sub_font)
    draw.text(((width - sw) / 2, 94 * scale), sub, font=japanese(13 * scale), fill=MUTED)
    # Arrow between the app (180, 210) and Applications (480, 210) icon slots.
    y = 210 * scale
    draw.line((262 * scale, y, 392 * scale, y), fill=SAGE, width=3 * scale)
    draw.polygon([(398 * scale, y), (384 * scale, y - 8 * scale), (384 * scale, y + 8 * scale)], fill=SAGE)
    OUT.mkdir(parents=True, exist_ok=True)
    name = "dmg-background.png" if scale == 1 else f"dmg-background@{scale}x.png"
    image.convert("RGB").save(OUT / name)


if __name__ == "__main__":
    nsis_sidebar()
    nsis_header()
    wix_banner()
    wix_dialog()
    dmg_background(1)
    dmg_background(2)
    print("Installer art written to", OUT)
