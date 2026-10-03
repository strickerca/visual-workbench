"""Independent verifier tests; no credentials, network or image-model calls."""

import hashlib
import importlib.util
import json
from pathlib import Path
import random
import struct
import tempfile
import unittest
from PIL import Image, ImageCms

SPEC = importlib.util.spec_from_file_location("exterior", Path(__file__).resolve().parents[1] / "verify_exterior.py")
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)


class IndependentProofTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="vw-ai-proof-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.width, self.height, self.radius = 19, 17, 3
        w, h = self.width, self.height
        self.mask = bytes(255 if 8 <= x < 11 and 7 <= y < 10 else 0 for y in range(h) for x in range(w))
        source = Image.new("RGBA", (w, h), (23, 87, 130, 255))
        result = source.copy()
        for y in range(7, 10):
            for x in range(8, 11):
                result.putpixel((x, y), (90, 120, 210, 255))
        self.profile = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
        source.save(self.root / "source.png", icc_profile=self.profile)
        result.save(self.root / "composite.png", icc_profile=self.profile)
        mask_image = Image.frombytes("RGBA", (w, h), bytes(channel for value in self.mask for channel in (0, 0, 0, 255 - value)))
        mask_image.save(self.root / "mask.png")
        source.save(self.root / "request-image.png")
        mask_image.save(self.root / "request-mask.png")
        (self.root / "original.input").write_bytes((self.root / "source.png").read_bytes())
        self.description = dict(feather_px=2, source_file_sha256=self.hash_file("original.input"),
            source_icc_sha256=None, normalized_source_icc_sha256=hashlib.sha256(self.profile).hexdigest(),
            source_pixels_sha256=VERIFY.pixel_hash(w, h, source.tobytes()), mask_sha256=hashlib.sha256(self.mask).hexdigest(),
            request_image_sha256=self.hash_file("request-image.png"), request_mask_sha256=self.hash_file("request-mask.png"))
        self.proof = dict(schema=1, width=w, height=h, dilation_px=3, dilation_metric="euclidean_disk_pixel_centers",
            source_file_sha256=self.description["source_file_sha256"], mask_sha256=self.description["mask_sha256"],
            source_pixels_sha256=self.description["source_pixels_sha256"], result_pixels_sha256=VERIFY.pixel_hash(w, h, result.tobytes()))
        prefix = b"vw-ai-spike/outside/v1\0" + struct.pack("<III", w, h, 3) + self.description["mask_sha256"].encode()
        before, after = hashlib.sha256(prefix), hashlib.sha256(prefix)
        outside = changed = 0
        for y in range(h):
            for x in range(w):
                if not self.brute_selected(self.mask, w, h, x, y, 3):
                    index = y * w + x
                    a, b = bytes(source.getpixel((x, y))), bytes(result.getpixel((x, y)))
                    before.update(struct.pack("<Q", index)); before.update(a)
                    after.update(struct.pack("<Q", index)); after.update(b)
                    outside += 1
                    changed += a != b
        self.proof.update(changed_outside=changed, outside_pixels=outside,
                          outside_sha256_before=before.hexdigest(), outside_sha256_after=after.hexdigest())
        self.write_records()

    def hash_file(self, name):
        return hashlib.sha256((self.root / name).read_bytes()).hexdigest()

    def write_records(self):
        identity = hashlib.sha256(json.dumps(self.description, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        (self.root / "prepared.json").write_text(json.dumps(dict(request_id=identity, description=self.description)), encoding="utf-8")
        (self.root / "proof.json").write_text(json.dumps(self.proof), encoding="utf-8")

    @staticmethod
    def brute_selected(mask, w, h, x, y, radius):
        return any(mask[yy * w + xx] and (xx - x) ** 2 + (yy - y) ** 2 <= radius ** 2
                   for yy in range(max(0, y - radius), min(h, y + radius + 1))
                   for xx in range(max(0, x - radius), min(w, x + radius + 1)))

    def test_complete_bundle_passes_with_nonzero_inside_change(self):
        result = VERIFY.verify(self.root)
        self.assertEqual(result["changed_outside"], 0)
        self.assertGreater(result["outside_pixels"], 0)
        self.assertFalse(result["provider_semantics_verified"])

    def test_bitset_disk_matches_independent_brute_oracle(self):
        rng = random.Random(0x56570011)
        for _ in range(30):
            w, h = rng.randrange(1, 15), rng.randrange(1, 15)
            mask = bytes(255 if rng.randrange(7) == 0 else 0 for _ in range(w * h))
            for radius in range(5):
                rows = list(VERIFY.dilated_rows(mask, w, h, radius))
                for y in range(h):
                    for x in range(w):
                        self.assertEqual(bool((rows[y] >> x) & 1), self.brute_selected(mask, w, h, x, y, radius))

    def test_forged_zero_count_cannot_hide_changed_exterior(self):
        with Image.open(self.root / "composite.png") as image:
            changed = image.copy()
        changed.putpixel((0, 0), (23, 87, 130, 254))
        changed.save(self.root / "composite.png", icc_profile=self.profile)
        self.proof["result_pixels_sha256"] = VERIFY.pixel_hash(self.width, self.height, changed.tobytes())
        self.write_records()
        with self.assertRaises(ValueError):
            VERIFY.verify(self.root)

    def test_original_replacement_fails_its_immutable_binding(self):
        (self.root / "original.input").write_bytes(b"replaced original")
        with self.assertRaises(ValueError):
            VERIFY.verify(self.root)

    def test_mask_replacement_cannot_expand_the_allowed_exterior(self):
        Image.new("RGBA", (self.width, self.height), (0, 0, 0, 0)).save(self.root / "mask.png")
        with self.assertRaises(ValueError):
            VERIFY.verify(self.root)

    def test_unknown_metric_and_inflated_radius_fail_closed(self):
        for field, value in (("dilation_metric", "square"), ("dilation_px", 65), ("dilation_px", -1)):
            original = self.proof[field]
            self.proof[field] = value
            self.write_records()
            with self.assertRaises(ValueError):
                VERIFY.verify(self.root)
            self.proof[field] = original

    def test_request_identity_is_recomputed(self):
        value = json.loads((self.root / "prepared.json").read_text())
        value["description"]["feather_px"] = 9
        (self.root / "prepared.json").write_text(json.dumps(value))
        with self.assertRaises(ValueError):
            VERIFY.verify(self.root)

    def test_wrong_dimensions_and_non_rgba_png_reject(self):
        for image in (Image.new("RGBA", (1, 1)), Image.new("RGB", (self.width, self.height))):
            image.save(self.root / "composite.png")
            with self.assertRaises(ValueError):
                VERIFY.verify(self.root)

    def test_profile_only_change_or_removal_cannot_pass_pixel_proof(self):
        with Image.open(self.root / "composite.png") as image:
            original = image.copy()
        for profile in (None, ImageCms.ImageCmsProfile(ImageCms.createProfile("XYZ")).tobytes()):
            original.save(self.root / "composite.png", icc_profile=profile)
            with self.assertRaises(ValueError):
                VERIFY.verify(self.root)

    def test_assumed_srgb_normalized_profile_is_bound_even_if_both_pngs_change(self):
        replacement = ImageCms.ImageCmsProfile(ImageCms.createProfile("XYZ")).tobytes()
        for name in ("source.png", "composite.png"):
            with Image.open(self.root / name) as image:
                original = image.copy()
            original.save(self.root / name, icc_profile=replacement)
        with self.assertRaises(ValueError):
            VERIFY.verify(self.root)


if __name__ == "__main__":
    unittest.main()
