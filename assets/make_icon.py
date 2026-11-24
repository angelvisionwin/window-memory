"""Genera assets/app.ico para Window Memory.

El icono representa la idea del programa: dos ventanas superpuestas (la de
delante y la recordada detrás) y una flecha circular de restauración.

Requiere Pillow:  python -m pip install pillow
Uso:              python assets/make_icon.py
"""

from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024
OUTPUT = Path(__file__).with_name("app.ico")
SIZES = [(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (128, 128), (256, 256)]

TILE_TOP = (94, 158, 255)
TILE_BOTTOM = (29, 78, 216)
TITLE_BAR = (226, 236, 253)
WINDOW = (255, 255, 255)
BORDER = (198, 216, 245)
BADGE = (22, 163, 74)
SHADOW = (9, 30, 76, 90)


def rounded_mask(size: int, radius: int, inset: int = 0) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [inset, inset, size - 1 - inset, size - 1 - inset], radius=radius, fill=255
    )
    return mask


def rounded_top_rect(draw: ImageDraw.ImageDraw, box, radius: int, fill) -> None:
    """Rectángulo redondeado solo arriba (barra de título)."""
    left, top, right, bottom = box
    draw.rounded_rectangle([left, top, right, bottom + radius], radius=radius, fill=fill)
    draw.rectangle([left, top + radius, right, bottom], fill=fill)


def draw_window(draw: ImageDraw.ImageDraw, box, radius: int, alpha: int) -> None:
    left, top, right, bottom = box
    draw.rounded_rectangle([left, top, right, bottom], radius=radius, fill=(255, 255, 255, alpha))
    draw.rounded_rectangle(
        [left, top, right, bottom],
        radius=radius,
        outline=(255, 255, 255, min(255, alpha + 60)),
        width=12,
    )


def arrow_head(draw: ImageDraw.ImageDraw, center, radius: float, angle: float, length: float, width: float) -> None:
    """Punta de flecha tangente al círculo, en el sentido de las agujas del reloj."""
    radians = math.radians(angle)
    cos, sin = math.cos(radians), math.sin(radians)
    px, py = center[0] + radius * cos, center[1] + radius * sin
    tangent = (-sin, cos)
    normal = (cos, sin)
    tip = (px + tangent[0] * length, py + tangent[1] * length)
    back = (px - tangent[0] * length * 0.35, py - tangent[1] * length * 0.35)
    draw.polygon(
        [
            tip,
            (back[0] + normal[0] * width, back[1] + normal[1] * width),
            (back[0] - normal[0] * width, back[1] - normal[1] * width),
        ],
        fill=(255, 255, 255, 255),
    )


def build() -> Image.Image:
    image = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))

    # Fondo: cuadrado redondeado con degradado azul.
    tile = Image.new("RGBA", (SIZE, SIZE))
    gradient = Image.new("RGB", (SIZE, SIZE))
    strip = Image.new("RGB", (1, SIZE))
    for y in range(SIZE):
        t = y / (SIZE - 1)
        strip.putpixel((0, y), tuple(round(TILE_TOP[c] + (TILE_BOTTOM[c] - TILE_TOP[c]) * t) for c in range(3)))
    gradient.paste(strip.resize((SIZE, SIZE)), (0, 0))
    tile.paste(gradient, (0, 0), rounded_mask(SIZE, 210, inset=40))
    image.alpha_composite(tile)

    draw = ImageDraw.Draw(image)

    # Ventana recordada, detrás y desplazada.
    draw_window(draw, [318, 196, 842, 610], 44, 72)

    # Sombra de la ventana principal.
    shadow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle([176, 356, 726, 872], radius=52, fill=SHADOW)
    image.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(22)))
    draw = ImageDraw.Draw(image)

    # Ventana principal: marco, barra de título y línea de separación.
    draw.rounded_rectangle([170, 340, 720, 856], radius=52, fill=WINDOW)
    rounded_top_rect(draw, [170, 340, 720, 470], 52, TITLE_BAR)
    draw.rectangle([170, 466, 720, 474], fill=BORDER)

    # Puntos de la barra de título.
    for index in range(3):
        cx = 226 + index * 62
        draw.ellipse([cx - 14, 390, cx + 14, 418], fill=TILE_BOTTOM)

    # Distintivo de restauración: anillo blanco, círculo verde y flecha circular.
    center = (798, 798)
    outer = 116
    inner = 100
    draw.ellipse(
        [center[0] - outer, center[1] - outer, center[0] + outer, center[1] + outer],
        fill=(255, 255, 255, 255),
    )
    draw.ellipse(
        [center[0] - inner, center[1] - inner, center[0] + inner, center[1] + inner],
        fill=BADGE,
    )
    arc_radius = 58
    draw.arc(
        [
            center[0] - arc_radius,
            center[1] - arc_radius,
            center[0] + arc_radius,
            center[1] + arc_radius,
        ],
        start=336,
        end=292,
        fill=(255, 255, 255, 255),
        width=24,
    )
    arrow_head(draw, center, arc_radius + 4, 334, 42, 26)

    return image


def main() -> None:
    image = build()
    image.save(OUTPUT, format="ICO", sizes=SIZES)
    print(f"escrito {OUTPUT} ({OUTPUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
