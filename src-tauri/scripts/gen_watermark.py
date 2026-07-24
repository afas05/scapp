"""Generate the recording watermark asset: a semi-transparent gray pill/oval
with "streamsnipe.live" inside, transparent background.

Regenerate with:  python src-tauri/scripts/gen_watermark.py
Output:           src-tauri/watermark.png

The PNG is overlaid bottom-right onto recordings via `gdkpixbufoverlay`
(see src-tauri/src/gstreamer.rs). If the dimensions change, update the
offset constants in start_recording().
"""
import os
from PIL import Image, ImageDraw, ImageFont

TEXT = "streamsnipe.live"

# Supersample for crisp anti-aliased edges, then downscale.
SCALE = 4
PAD_X, PAD_Y = 34, 18          # padding around the text inside the pill (base px)
BASE_FONT = 30                 # base font size (px)

# Colors (RGBA). Whole asset is semi-transparent; gdkpixbufoverlay `alpha`
# blends it further at runtime.
PILL_FILL = (70, 74, 82, 150)      # gray, ~59% alpha
PILL_STROKE = (255, 255, 255, 40)  # faint light rim
TEXT_FILL = (255, 255, 255, 205)   # ~80% alpha white


def load_font(size):
    for name in ("segoeui.ttf", "arial.ttf", "DejaVuSans.ttf", "Verdana.ttf"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default()


def main():
    s = SCALE
    font = load_font(BASE_FONT * s)

    # Measure text.
    tmp = Image.new("RGBA", (10, 10))
    d = ImageDraw.Draw(tmp)
    l, t, r, b = d.textbbox((0, 0), TEXT, font=font)
    tw, th = r - l, b - t

    w = tw + 2 * PAD_X * s
    h = th + 2 * PAD_Y * s

    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Pill = fully-rounded rectangle (radius = half height) → oval/stadium.
    radius = h // 2
    draw.rounded_rectangle([0, 0, w - 1, h - 1], radius=radius,
                           fill=PILL_FILL, outline=PILL_STROKE, width=max(1, s))

    # Center the text.
    tx = (w - tw) // 2 - l
    ty = (h - th) // 2 - t
    draw.text((tx, ty), TEXT, font=font, fill=TEXT_FILL)

    # Downscale to final size.
    final = img.resize((w // s, h // s), Image.LANCZOS)

    out = os.path.join(os.path.dirname(__file__), "..", "watermark.png")
    out = os.path.abspath(out)
    final.save(out)
    print(f"wrote {out}  ({final.width}x{final.height})")


if __name__ == "__main__":
    main()
