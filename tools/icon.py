"""Export the Pulse mark to Android launcher, notification, and store assets."""

from pathlib import Path
from PIL import Image, ImageDraw

SOURCE = Path("app/icon-src/pulse-mark.png")
RES = Path("app/src/main/res")
PLATE = "#101719"
DENSITIES = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}


def fit(source, size, mark_fraction):
    mark = source.copy()
    target = round(size * mark_fraction)
    mark.thumbnail((target, target), Image.Resampling.LANCZOS)
    image = Image.new("RGBA", (size, size))
    image.alpha_composite(mark, ((size - mark.width) // 2, (size - mark.height) // 2))
    return image


def plate(source, size, rounded=False):
    artwork = fit(source, size, 0.69)
    image = Image.new("RGBA", (size, size), PLATE)
    image.alpha_composite(artwork)
    if rounded:
        mask = Image.new("L", (size * 3, size * 3))
        ImageDraw.Draw(mask).ellipse((0, 0, mask.width - 1, mask.height - 1), fill=255)
        image.putalpha(mask.resize((size, size), Image.Resampling.LANCZOS))
    return image


def silhouette(artwork):
    white = Image.new("RGBA", artwork.size, "white")
    white.putalpha(artwork.getchannel("A"))
    return white


def main():
    source = Image.open(SOURCE).convert("RGBA")
    bounds = source.getchannel("A").point(lambda value: 255 if value > 64 else 0).getbbox()
    if bounds is None:
        raise ValueError("The source has no visible mark")
    mark = source.crop(bounds)

    for density, scale in DENSITIES.items():
        folder = RES / f"mipmap-{density}"
        size = round(108 * scale)
        fit(mark, size, 0.58).save(folder / "ic_launcher_foreground.png")
        plate(mark, round(48 * scale)).save(folder / "ic_launcher.png")
        plate(mark, round(48 * scale), rounded=True).save(folder / "ic_launcher_round.png")

    drawable = RES / "drawable-nodpi"
    drawable.mkdir(exist_ok=True)
    silhouette(fit(mark, 432, 0.58)).save(drawable / "ic_launcher_monochrome.png")
    silhouette(fit(mark, 192, 0.88)).save(drawable / "ic_app_mark.png")
    plate(mark, 512).save("fastlane/metadata/android/en-US/images/icon.png")
    plate(mark, 512, rounded=True).save("docs/pulse.png")
    plate(mark, 256, rounded=True).save("web/public/pulse.png")


if __name__ == "__main__":
    main()
