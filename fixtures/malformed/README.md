`truncated.jpg`, `truncated.png`, `declared-size-bomb.png`, `corrupt-xref.pdf` and
`billion-laughs.svg` are intentionally hostile importer inputs. Generation and
hash checks do not decode or expand them. See `../manifest.json` for provenance,
hashes and expected rejection/repair cases. Passing a hash check is not parser safety.
