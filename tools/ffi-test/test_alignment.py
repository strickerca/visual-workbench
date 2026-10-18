import unittest

from check_alignment import check_load_alignment


class AlignmentTests(unittest.TestCase):
    def test_every_load_segment_must_meet_16k(self):
        self.assertEqual(check_load_alignment(
            "  LOAD off 0x0 vaddr 0x0 align 2**14\n"
            "       filesz 0x80 memsz 0x80 flags r--\n"
            "  LOAD off 0x4000 vaddr 0x4000 align 2**16\n"), 2)
        with self.assertRaisesRegex(ValueError, "below 16 KiB"):
            check_load_alignment(
                "LOAD off 0x0 align 2**14\nLOAD off 0x4000 align 2**12\n")

    def test_absent_or_unrecognized_load_fails_closed(self):
        for output in ("", "PE executable", "LOAD off 0x0 align 16384",
                       "LOAD off 0x0 align 2**-1", "LOAD off 0x0 align 2**64",
                       "LOAD off 0x0 align 2**14 unparsed"):
            with self.subTest(output=output), self.assertRaises(ValueError):
                check_load_alignment(output)


if __name__ == "__main__":
    unittest.main()
