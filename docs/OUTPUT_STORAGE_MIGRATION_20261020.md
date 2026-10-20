# Persist explicit output frames

The model's explicit `output` layer was rejected by SQLite's schema2 CHECK,
preventing output-frame projects from persisting. Schema3 admits that existing
model convention. Existing schema1/2 databases upgrade forward, with schema2
canonical state, checkpoint, accepted log, assets, pending edits and projections
validated inside the same Immediate transaction before any DDL.

The migration copies all ten layer and fifteen object columns, replaces child
then parent tables, and recreates both indexes. Foreign-key enforcement stays
on. Schema, FK and integrity checks precede commit; malformed historical data
rolls back without changing rows or version. Old schema fingerprints remain
exact001+002, independently of the fresh schema3 reference. Older applications
refuse the newer schema instead of ignoring output settings.

Actual cloud checks pass55 selected storage cases and11 affected file
import/export cases. Tests cover nonempty historical rows, exact checkpoint/log
and pending bytes, ten corruptions, output-frame commit/reopen/idempotent retry,
and an archive roundtrip. Astra source review found no remaining issue. See
[the source-bound receipt](evidence/OUTPUT_STORAGE_MIGRATION_20261020.json).

Windows/Android artifact and device validation remain outside this receipt.
