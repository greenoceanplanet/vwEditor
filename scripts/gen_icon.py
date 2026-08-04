"""assets/vweditor.ico 를 만든다. 아이콘을 고치려면 이 파일을 고치고 다시 돌린다.

    py -m pip install pillow
    py scripts/gen_icon.py

디자인: 흰 바탕 위 데이터 그리드(파란 헤더 + 열/행 구분선)에 "vw" 워드마크.

두 가지가 의도적이다:
  * 큰 크기(>=48px)는 4배로 그린 뒤 축소하지만, **16/32px는 1:1로 따로 그린다.**
    축소하면 격자선이 뭉개져 얼룩이 된다. 16px에서는 글자가 읽히지 않으므로
    워드마크를 아예 빼고 격자 구조만 남긴다.
  * 워드마크 자리에는 격자선을 긋지 않는다(draw_large 의 band_top/band_bot).
    선 위에 흰 테두리를 씌워 가리면 선이 갉아먹힌 것처럼 파여 보인다.
"""
from PIL import Image, ImageDraw, ImageFont
import os

HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(os.path.dirname(HERE), "assets")

BLUE = (0, 120, 215)
BLUE_DK = (0, 86, 160)
WHITE = (255, 255, 255)
LINE = (176, 186, 200)
BORDER = (150, 162, 178)

# 평범한 고딕 볼드면 무엇이든 된다. OS별로 먼저 찾히는 것을 쓴다.
FONT_CANDIDATES = [
    "C:/Windows/Fonts/arialbd.ttf",
    "C:/Windows/Fonts/segoeuib.ttf",
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
]
FONT = next((p for p in FONT_CANDIDATES if os.path.exists(p)), None)
if FONT is None:
    raise SystemExit(
        "고딕 볼드 폰트를 찾지 못했다. FONT_CANDIDATES 에 경로를 추가하라."
    )

SS = 4  # 큰 크기용 슈퍼샘플 배율


def draw_large(d, S):
    """Full detail: 3x4 grid + wordmark. Used for >=48px."""
    r = S * 0.18
    d.rounded_rectangle((0, 0, S - 1, S - 1), radius=r, fill=WHITE,
                        outline=BORDER, width=max(1, int(S * 0.018)))

    m = S * 0.14
    w = S - 2 * m
    hb = m + w * 0.185

    d.rounded_rectangle((m, m, m + w, hb), radius=S * 0.05, fill=BLUE)
    d.rectangle((m, hb - S * 0.05, m + w, hb), fill=BLUE)

    lw = max(1, int(S * 0.016))
    cw = w / 3.0

    # The wordmark occupies a clear band across the middle. Grid lines stop at
    # its edges instead of running underneath — a white halo over a line leaves
    # visible notches, so we simply don't draw the line there.
    band_top = m + w * 0.42
    band_bot = m + w * 0.78

    for i in (1, 2):
        x = m + cw * i
        d.line((x, m, x, band_top), fill=LINE, width=lw)
        d.line((x, band_bot, x, m + w), fill=LINE, width=lw)
    for i in range(1, 4):
        y = hb + (m + w - hb) * i / 4
        if band_top < y < band_bot:
            continue
        d.line((m, y, m + w, y), fill=LINE, width=lw)
    d.rectangle((m, m, m + w, m + w), outline=LINE, width=lw)


def overlay_wordmark(img, S):
    """Wordmark centred in the clear band. No halo needed — nothing behind it."""
    layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    f = ImageFont.truetype(FONT, int(S * 0.44))
    text = "vw"
    m, w = S * 0.14, S - 2 * S * 0.14
    box = d.textbbox((0, 0), text, font=f)
    tw, th = box[2] - box[0], box[3] - box[1]
    tx = S * 0.5 - tw / 2 - box[0]
    ty = (m + w * 0.42 + m + w * 0.78) / 2 - th / 2 - box[1]
    d.text((tx, ty), text, font=f, fill=BLUE_DK)
    return Image.alpha_composite(img, layer)


def render_large(size):
    S = size * SS
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    draw_large(ImageDraw.Draw(img, "RGBA"), S)
    img = overlay_wordmark(img, S)
    return img.resize((size, size), Image.LANCZOS)


def render_small(size):
    """Hand-drawn at 1:1 for 16/32px. Pixel-snapped, simplified."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    px = lambda v: int(round(v))
    inset = 1 if size >= 32 else 0
    x0, y0, x1, y1 = inset, inset, size - 1 - inset, size - 1 - inset

    d.rectangle((x0, y0, x1, y1), fill=WHITE, outline=BORDER)
    hb = y0 + px((y1 - y0) * 0.30)
    d.rectangle((x0 + 1, y0 + 1, x1 - 1, hb), fill=BLUE)

    if size >= 32:
        # Column dividers run only through the header strip; the body below is
        # left clear for the wordmark (same reason as draw_large's band).
        xm = px((x0 + x1) / 2)
        d.line((xm, y0 + 1, xm, hb), fill=LINE)
        f = ImageFont.truetype(FONT, px(size * 0.42))
        t = "vw"
        b = d.textbbox((0, 0), t, font=f)
        d.text(((size - (b[2] - b[0])) / 2 - b[0],
                hb + (y1 - hb) / 2 - (b[3] - b[1]) / 2 - b[1] + 1),
               t, font=f, fill=BLUE_DK)
    else:
        # 16px: wordmark is illegible — show grid structure only
        for i in (1, 2):
            x = x0 + px((x1 - x0) * i / 3)
            d.line((x, y0 + 1, x, y1 - 1), fill=LINE)
        for i in (1, 2):
            y = hb + px((y1 - hb) * i / 3)
            d.line((x0 + 1, y, x1 - 1, y), fill=LINE)
    return img


SIZES = [256, 128, 64, 48, 32, 16]


def build(size):
    return render_small(size) if size <= 32 else render_large(size)


if __name__ == "__main__":
    os.makedirs(ASSETS, exist_ok=True)
    imgs = [build(s) for s in SIZES]

    # ICO 는 각 크기의 이미지를 따로 담는다. Pillow 는 첫 이미지에서 축소본을
    # 만들지만, 여기서는 16/32 를 손으로 그렸으므로 append_images 로 넘긴다.
    ico = os.path.join(ASSETS, "vweditor.ico")
    imgs[0].save(
        ico,
        format="ICO",
        sizes=[(s, s) for s in SIZES],
        append_images=imgs[1:],
    )
    # 창 아이콘·README 용 PNG(256px).
    imgs[0].save(os.path.join(ASSETS, "vweditor.png"))
    print(f"wrote {ico} ({', '.join(str(s) for s in SIZES)})")
