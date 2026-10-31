# Roon Beacon logo

`roon-beacon-original.png` is the user-supplied artwork, preserved byte-for-byte. Do not redraw or replace it with a generated logo.

The PNGs in `web/public/` are transparent, centered derivatives for the interface, favicon, and Apple touch icon. Only transparent outer padding is removed before proportional resizing. With ImageMagick:

```sh
magick assets/branding/roon-beacon-original.png -trim +repage \
  -resize 256x256 -background none -gravity center -extent 256x256 \
  -strip -define png:compression-level=9 PNG32:web/public/logo.png
```

Use 64 pixels for `favicon.png` and 180 pixels for `apple-touch-icon.png`. Rebuild the frontend to copy these assets into the Rust embedded resources.
