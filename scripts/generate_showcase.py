import os
import math
from PIL import Image, ImageDraw, ImageFont, ImageFilter

W, H = 2560, 1440

# 1. Base gradient background
bg = Image.new("RGBA", (W, H), (0, 0, 0, 255))
bg_draw = ImageDraw.Draw(bg)

# Vertical smooth dark luxury gradient: #08090e -> #0f111a
for y in range(H):
    r = int(8 + (15 - 8) * (y / H))
    g = int(9 + (17 - 9) * (y / H))
    b = int(14 + (26 - 14) * (y / H))
    bg_draw.line([(0, y), (W, y)], fill=(r, g, b, 255))

# Ambient soft glow orbs behind devices
glow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
glow_draw = ImageDraw.Draw(glow)

# Violet orb behind left devices
glow_draw.ellipse([150, 320, 1050, 1180], fill=(124, 58, 237, 45))
# Cyan/Indigo orb behind right devices
glow_draw.ellipse([1450, 360, 2350, 1220], fill=(14, 165, 233, 40))
# Central subtle purple ambient
glow_draw.ellipse([880, 240, 1720, 960], fill=(99, 102, 241, 35))

glow = glow.filter(ImageFilter.GaussianBlur(140))
bg = Image.alpha_composite(bg, glow)

# Fonts
font_dir = r"C:\Windows\Fonts"
font_title = ImageFont.truetype(os.path.join(font_dir, "segoeuib.ttf"), 54)
font_badge = ImageFont.truetype(os.path.join(font_dir, "segoeuib.ttf"), 17)
font_subtitle = ImageFont.truetype(os.path.join(font_dir, "segoeui.ttf"), 22)
font_card_title = ImageFont.truetype(os.path.join(font_dir, "segoeuib.ttf"), 22)
font_card_sub = ImageFont.truetype(os.path.join(font_dir, "segoeui.ttf"), 15)
font_pill = ImageFont.truetype(os.path.join(font_dir, "segoeuib.ttf"), 15)

draw = ImageDraw.Draw(bg)

# 2. Header: Icon, Title, Badge, Subtitle
icon_path = "client/public/icons/pierogi-dark-512.png"
if os.path.exists(icon_path):
    icon = Image.open(icon_path).convert("RGBA").resize((76, 76), Image.Resampling.LANCZOS)
    mask = Image.new("L", (76, 76), 0)
    mask_draw = ImageDraw.Draw(mask)
    mask_draw.ellipse([0, 0, 75, 75], fill=255)
    bg.paste(icon, (140, 56), mask)

title_text = "ModularChat"
draw.text((235, 54), title_text, font=font_title, fill=(255, 255, 255, 255))

# Badge next to title
badge_text = "PIEROGI EDITION"
badge_bbox = font_badge.getbbox(badge_text)
badge_w = badge_bbox[2] - badge_bbox[0] + 28
badge_h = 32
badge_x = 235 + font_title.getbbox(title_text)[2] + 20
badge_y = 68

draw.rounded_rectangle([badge_x, badge_y, badge_x + badge_w, badge_y + badge_h], radius=16,
                       fill=(124, 58, 237, 45), outline=(167, 139, 250, 180), width=2)
draw.text((badge_x + 14, badge_y + 6), badge_text, font=font_badge, fill=(216, 180, 254, 255))

# Subtitle
sub_text = "Self-Hosted Character Chat • Multi-Provider 9Router • Encrypted Memory • Liquid Glass UI"
draw.text((238, 122), sub_text, font=font_subtitle, fill=(148, 163, 184, 255))

# 3. Device Mockup Geometry
DEVICE_W = 440
DEVICE_H = 976  # 440 x 976 matches 1082 x 2402 ratio
CORNER_RADIUS = 46
BEZEL = 10

X_START = 140
GAP = (W - 2 * X_START - 4 * (DEVICE_W + 2 * BEZEL)) // 3
Y_TOP = 205

screens = [
    {
        "file": "screen_01_home.png",
        "num": "01",
        "title": "Chat Vault & Home",
        "sub": "Encrypted AES-256 local storage, instant search & real-time PC server sync"
    },
    {
        "file": "screen_02_chat.png",
        "num": "02",
        "title": "Immersive Dialogue",
        "sub": "Live markdown streaming, branching turns & telemetry reasoning stats"
    },
    {
        "file": "screen_03_drawer.png",
        "num": "03",
        "title": "Liquid Glass Drawer",
        "sub": "Spring physics menu, per-chat personas & custom world lore (.txt)"
    },
    {
        "file": "screen_04_models.png",
        "num": "04",
        "title": "Model Catalog & Presets",
        "sub": "9Router multi-route presets with dedicated thinking effort control"
    }
]

# Create shadow layer for all devices
shadow_layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
shadow_draw = ImageDraw.Draw(shadow_layer)

device_positions = []
for i, item in enumerate(screens):
    dx = X_START + i * (DEVICE_W + 2 * BEZEL + GAP)
    dy = Y_TOP
    device_positions.append((dx, dy))
    # Deep ambient drop shadow
    shadow_draw.rounded_rectangle(
        [dx + BEZEL, dy + BEZEL + 24, dx + DEVICE_W + BEZEL, dy + DEVICE_H + BEZEL + 24],
        radius=CORNER_RADIUS,
        fill=(0, 0, 0, 180)
    )

shadow_layer = shadow_layer.filter(ImageFilter.GaussianBlur(32))
bg = Image.alpha_composite(bg, shadow_layer)
draw = ImageDraw.Draw(bg)

# Composite each device frame and screenshot
for i, item in enumerate(screens):
    dx, dy = device_positions[i]
    outer_box = [dx, dy, dx + DEVICE_W + 2 * BEZEL, dy + DEVICE_H + 2 * BEZEL]
    inner_box = [dx + BEZEL, dy + BEZEL, dx + BEZEL + DEVICE_W, dy + BEZEL + DEVICE_H]

    # Outer titanium bezel frame
    draw.rounded_rectangle(outer_box, radius=CORNER_RADIUS + BEZEL, fill=(24, 25, 32, 255),
                           outline=(71, 74, 90, 180), width=2)
    # Subtle inner bezel rim
    draw.rounded_rectangle([dx + 3, dy + 3, dx + DEVICE_W + 2 * BEZEL - 3, dy + DEVICE_H + 2 * BEZEL - 3],
                           radius=CORNER_RADIUS + BEZEL - 3, outline=(39, 41, 52, 255), width=2)

    # Load and resize screen image
    if os.path.exists(item["file"]):
        raw_screen = Image.open(item["file"]).convert("RGBA")
        resized_screen = raw_screen.resize((DEVICE_W, DEVICE_H), Image.Resampling.LANCZOS)

        # Rounded corner mask for screen
        mask = Image.new("L", (DEVICE_W, DEVICE_H), 0)
        mask_draw = ImageDraw.Draw(mask)
        mask_draw.rounded_rectangle([0, 0, DEVICE_W - 1, DEVICE_H - 1], radius=CORNER_RADIUS, fill=255)

        bg.paste(resized_screen, (dx + BEZEL, dy + BEZEL), mask)

    # Glass highlight border on top of screen
    draw.rounded_rectangle(inner_box, radius=CORNER_RADIUS, outline=(255, 255, 255, 32), width=1)

    # Caption below device
    cap_y = dy + DEVICE_H + 2 * BEZEL + 20
    header_line = f"{item['num']}. {item['title']}"
    draw.text((dx + BEZEL, cap_y), header_line, font=font_card_title, fill=(241, 245, 249, 255))
    
    # Sub-caption with line-wrap if needed
    sub_words = item["sub"].split(" ")
    line1, line2 = "", ""
    for w in sub_words:
        test = (line1 + " " + w).strip()
        if font_card_sub.getbbox(test)[2] < DEVICE_W + 15:
            line1 = test
        else:
            line2 = (line2 + " " + w).strip()
    draw.text((dx + BEZEL, cap_y + 30), line1, font=font_card_sub, fill=(148, 163, 184, 255))
    if line2:
        draw.text((dx + BEZEL, cap_y + 52), line2, font=font_card_sub, fill=(148, 163, 184, 255))

# 4. Bottom feature pills (clean text with bullet dots)
pills = [
    "VAULT • AES-256-GCM / DPAPI",
    "MEMORY • Local llama.cpp GGUF",
    "ROUTING • 9Router Multi-Provider",
    "REASONING • Dedicated Effort Control",
    "DESIGN • Liquid Glass Physics UI"
]

pill_total_w = sum(font_pill.getbbox(p)[2] - font_pill.getbbox(p)[0] + 36 for p in pills) + (len(pills) - 1) * 16
cur_px = (W - pill_total_w) // 2
pill_y = H - 62

for p in pills:
    pb = font_pill.getbbox(p)
    pw = pb[2] - pb[0] + 36
    ph = 36
    draw.rounded_rectangle([cur_px, pill_y, cur_px + pw, pill_y + ph], radius=18,
                           fill=(30, 32, 45, 200), outline=(75, 80, 105, 120), width=1)
    draw.text((cur_px + 18, pill_y + 8), p, font=font_pill, fill=(203, 213, 225, 255))
    cur_px += pw + 16

# Save final images
os.makedirs("assets", exist_ok=True)
bg.convert("RGB").save("assets/showcase.png", quality=95, optimize=True)
bg.convert("RGB").save("showcase.png", quality=95, optimize=True)
print("Updated showcase sheet saved: assets/showcase.png and showcase.png")
