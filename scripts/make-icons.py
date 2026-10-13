"""Gera os ícones do app (Android + PWA) com Pillow: balão de conversa branco sobre fundo roxo."""
import os
import sys
from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RES = os.path.join(ROOT, 'client', 'android', 'app', 'src', 'main', 'res')
PUBLIC = os.path.join(ROOT, 'client', 'public')
PURPLE = (124, 58, 237, 255)


def bubble(size, pad_ratio, bg=None):
    """Balão com três pontos; pad_ratio controla a margem (zona segura do ícone adaptativo)."""
    S = size * 4  # supersampling
    img = Image.new('RGBA', (S, S), bg or (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    pad = S * pad_ratio
    x0, y0, x1, y1 = pad, pad * 1.1, S - pad, S - pad * 1.55
    d.rounded_rectangle((x0, y0, x1, y1), radius=(y1 - y0) * 0.32, fill=(255, 255, 255, 255))
    # speech-bubble tail
    tx = x0 + (x1 - x0) * 0.22
    d.polygon([(tx, y1 - 4), (tx + (x1 - x0) * 0.16, y1 - 4), (tx - (x1 - x0) * 0.02, y1 + (y1 - y0) * 0.30)], fill=(255, 255, 255, 255))
    r = (y1 - y0) * 0.075
    cy = (y0 + y1) / 2
    for k in (-1, 0, 1):
        cx = (x0 + x1) / 2 + k * (x1 - x0) * 0.2
        d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=PURPLE)
    return img.resize((size, size), Image.LANCZOS)


def full_icon(size, round_shape=False):
    bg = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    mask = Image.new('L', (size, size), 0)
    md = ImageDraw.Draw(mask)
    if round_shape:
        md.ellipse((0, 0, size - 1, size - 1), fill=255)
    else:
        md.rounded_rectangle((0, 0, size - 1, size - 1), radius=size * 0.22, fill=255)
    solid = Image.new('RGBA', (size, size), PURPLE)
    bg.paste(solid, (0, 0), mask)
    fg = bubble(size, 0.2)
    bg.alpha_composite(fg)
    return bg


def main():
    legacy = {'mdpi': 48, 'hdpi': 72, 'xhdpi': 96, 'xxhdpi': 144, 'xxxhdpi': 192}
    adaptive = {'mdpi': 108, 'hdpi': 162, 'xhdpi': 216, 'xxhdpi': 324, 'xxxhdpi': 432}
    for dens, px in legacy.items():
        d = os.path.join(RES, f'mipmap-{dens}')
        full_icon(px).save(os.path.join(d, 'ic_launcher.png'))
        full_icon(px, True).save(os.path.join(d, 'ic_launcher_round.png'))
        # adaptive icon foreground: the bubble inside the safe zone (66%)
        bubble(adaptive[dens], 0.30).save(os.path.join(d, 'ic_launcher_foreground.png'))
    with open(os.path.join(RES, 'values', 'ic_launcher_background.xml'), 'w', encoding='utf-8') as f:
        f.write('<?xml version="1.0" encoding="utf-8"?>\n<resources>\n    <color name="ic_launcher_background">#7C3AED</color>\n</resources>\n')
    full_icon(512).save(os.path.join(PUBLIC, 'icon-512.png'))
    full_icon(192).save(os.path.join(PUBLIC, 'icon-192.png'))
    print('icones gerados')


if __name__ == '__main__':
    sys.exit(main())
