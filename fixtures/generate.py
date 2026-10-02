"""Reproducible, first-party importer fixtures. Never render the hostile corpus here.

Run through tools/bench/image/run_pc.ps1 for bounded process-tree execution.
All output stays in the ignored generated directory; existing runs are preserved.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import struct
import subprocess
import zlib

import numpy as np
from PIL import Image, ImageDraw
from reportlab.pdfgen import canvas
from reportlab.lib.pdfencrypt import StandardEncryption
from reportlab.lib.utils import ImageReader
from pypdf import PdfReader

ROOT = Path(__file__).resolve().parents[1]
SEED = 200_16320
VERSIONS = {"numpy": "2.3.5", "Pillow": "12.1.0", "reportlab": "4.4.10", "pypdf": "6.8.0"}


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def png_chunk(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def ppm(path: Path, width: int, height: int, seed: int) -> None:
    """Bound pixel generation to 256 rows; no whole 200 MP Python array."""
    rng = np.random.Generator(np.random.PCG64(seed))
    stamp = Image.new("RGB", (800, 160), "white")
    ImageDraw.Draw(stamp).text((20, 30), f"VISUAL WORKBENCH {width} x {height} SYNTHETIC", fill="black", font_size=26)
    label = np.asarray(stamp)
    x = np.arange(width, dtype=np.int32)[None, :]
    with path.open("xb") as stream:
        stream.write(f"P6\n{width} {height}\n255\n".encode("ascii"))
        for top in range(0, height, 256):
            rows = min(256, height - top)
            y = np.arange(top, top + rows, dtype=np.int32)[:, None]
            pixels = np.empty((rows, width, 3), dtype=np.uint8)
            for channel in range(3):
                noise = rng.integers(-18, 19, size=(rows, width), dtype=np.int16)
                base = (x * (channel + 1) * 255 // width + y * (3 - channel) * 255 // height) % 256
                pixels[:, :, channel] = np.clip(base + noise, 0, 255)
            if top == 0:
                pixels[:160, :800] = label
            stream.write(pixels.tobytes())
            if top % 2048 == 0:
                print(f"pixels {width}x{height}: row {top}/{height}", flush=True)


def generate(destination: Path, vips: Path) -> dict:
    generated = (ROOT / "fixtures/generated").resolve()
    destination = destination.resolve()
    if destination.parent != generated or destination.exists():
        raise ValueError("Use a new direct child of fixtures/generated")
    for package, expected in VERSIONS.items():
        if importlib.metadata.version(package) != expected:
            raise ValueError(f"Fixture tool version differs: {package} requires {expected}")
    version = subprocess.check_output([str(vips), "--version"], text=True, timeout=15).strip()
    if version != "vips-8.18.7":
        raise ValueError("libvips version differs")
    destination.mkdir(parents=True)
    records = []

    def record(name, purpose, **metadata):
        path = destination / name
        records.append(dict(path=name, bytes=path.stat().st_size, sha256=sha256(path),
                            provenance="First-party synthetic fixture; fixtures/generate.py", 
                            license="LicenseRef-VisualWorkbench-Proprietary", purpose=purpose, **metadata))

    for name, w, h, offset in [("200mp", 16320, 12240, 0), ("12mp-a", 4000, 3000, 1),
                               ("12mp-b", 4000, 3000, 2), ("long", 1440, 20000, 3)]:
        source = destination / (name + ".ppm")
        ppm(source, w, h, SEED + offset)
        extension = "png" if name == "long" else "jpg"
        output = destination / (name + "." + extension)
        # The minimal libvips web build has no PNM loader. Pillow only opens our
        # just-created bounded PPM here; hostile corpus files never reach this path.
        previous_limit = Image.MAX_IMAGE_PIXELS
        try:
            Image.MAX_IMAGE_PIXELS = 200_000_000
            with Image.open(source) as image:
                if image.size != (w, h):
                    raise ValueError("Generated image dimensions differ")
                image.save(output, **({"quality": 95, "subsampling": 2} if extension == "jpg" else {}))
        finally:
            Image.MAX_IMAGE_PIXELS = previous_limit
        source.unlink()  # Exact intermediate created by this invocation, no recursive deletion.
        record(output.name, "noise, gradients and text", width=w, height=h, seed=SEED + offset,
               jpeg_quality=95 if extension == "jpg" else None)

    # A small own image embedded into PDFs; deterministic ReportLab IDs and dates.
    picture = destination / "pdf-image.png"
    Image.new("RGB", (64, 64), (42, 127, 193)).save(picture)
    record(picture.name, "generated blue image embedded in PDFs", width=64, height=64)
    for name in ("multipage.pdf", "password.pdf", "annotations.pdf"):
        encryption = StandardEncryption("vw-fixture", ownerPassword="vw-fixture-owner", strength=128) if name == "password.pdf" else None
        pdf = canvas.Canvas(str(destination / name), invariant=1, pageCompression=1, encrypt=encryption)
        pdf.setAuthor("Visual Workbench synthetic fixtures")
        for page in range(3):
            pdf.drawString(60, 760, f"Synthetic page {page + 1}: text / vector / raster")
            pdf.setFillColorRGB(0.1, 0.4, 0.7)
            pdf.rect(60, 520, 200, 180, fill=1)
            pdf.circle(360, 600, 70, fill=0)
            # ImageReader makes ReportLab name the XObject from pixel content,
            # not the machine-specific absolute input filename.
            pdf.drawImage(ImageReader(str(picture)), 60, 400, width=96, height=96)
            if name == "annotations.pdf":
                pdf.textAnnotation("Existing synthetic annotation", Rect=(280, 520, 360, 580))
            pdf.showPage()
        pdf.save()
        reader = PdfReader(destination / name)
        if reader.is_encrypted:
            if not reader.decrypt("vw-fixture"):
                raise ValueError("Fixture password verification failed")
        if len(reader.pages) != 3 or (name == "annotations.pdf" and not all(p.get("/Annots") for p in reader.pages)):
            raise ValueError("Fixture PDF structure verification failed")
        record(name, "3 pages with text, vectors, raster" + ("; password vw-fixture" if encryption else ""),
               pages=3, encrypted=bool(encryption), existing_annotations=name == "annotations.pdf")

    bodies = {
        "simple.svg": '<rect width="64" height="64" fill="#287fc1"/>',
        "text.svg": '<text x="4" y="32">Synthetic text</text>',
        "script.svg": '<script>throw new Error("SVG script must never execute")</script>',
        "external.svg": '<image href="http://example.invalid/fixture.png"/><image href="file:///vw-nonexistent-fixture.png"/>',
    }
    for name, body in bodies.items():
        (destination / name).write_text('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">' + body + '</svg>\n', encoding="utf-8")
        record(name, "SVG sanitizer fixture; never execute scripts or resolve external references")
    (destination / "huge-viewbox.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000000000 1000000000"><rect width="1" height="1"/></svg>\n', encoding="utf-8")
    record("huge-viewbox.svg", "SVG bounds/resource rejection")
    for src, name in [("200mp.jpg", "truncated.jpg"), ("long.png", "truncated.png")]:
        with (destination / src).open("rb") as stream:
            (destination / name).write_bytes(stream.read(100))
        record(name, "malformed truncated input; expected rejection", hostile=True)
    bomb = b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 1000000000, 1000000000, 8, 2, 0, 0, 0)) + png_chunk(b"IDAT", zlib.compress(b"\0")) + png_chunk(b"IEND", b"")
    (destination / "declared-size-bomb.png").write_bytes(bomb)
    record("declared-size-bomb.png", "tiny payload with enormous declared dimensions; must reject before allocation", hostile=True)
    pdf = (destination / "multipage.pdf").read_bytes()
    start = pdf.rfind(b"startxref\n")
    if start < 0:
        raise ValueError("Generated PDF xref not found")
    (destination / "corrupt-xref.pdf").write_bytes(pdf[:start] + b"startxref\n999999999999\n%%EOF\n")
    record("corrupt-xref.pdf", "invalid cross-reference offset; repair/rejection test", hostile=True)
    entities = ['<!ENTITY e0 "ha">'] + [f'<!ENTITY e{i} "' + f'&e{i-1};' * 10 + '">' for i in range(1, 10)]
    xml = '<!DOCTYPE svg [' + ''.join(entities) + ']><svg xmlns="http://www.w3.org/2000/svg"><text>&e9;</text></svg>'
    (destination / "billion-laughs.svg").write_text(xml, encoding="utf-8")
    record("billion-laughs.svg", "entity expansion attack; reject DOCTYPE before parsing", hostile=True)
    manifest = dict(schema=1, generator_sha256=sha256(Path(__file__)), seed=SEED, tools={**VERSIONS, "libvips": version},
                    files=records, malformed_files_rendered=False)
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"Fixture generation complete: {len(records)} files; hostile files written only", flush=True)
    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--vips", type=Path, required=True)
    args = parser.parse_args()
    generate(args.output, args.vips)
