# Checked package geometry for verification

Root adds the following allocation-free native package helpers in its own
verification_geometry leaf. P/V active files, agent protocol and package schema
are unchanged:

```rust
pub fn verification_image_geometry(package: &Package)
    -> Result<VerificationImageGeometry>;
pub fn verification_comparison_transform(
    before: VerificationImageGeometry,
    after: VerificationImageGeometry,
    translation_document: vw_geom::Point,
) -> Result<vw_geom::Affine>;
```

VerificationImageGeometry carries source_size, output_size, actual padded
image_size, content_size, crop_document, document_to_image, image_to_document and
has_output_frame. The input is the existing checked Package, whose constructor
or compiler validates its PNG/hash/inventory and canonical overview mapping.
The helper checks the clean descriptor and scalar/mapping consistency again,
uses ImageOutputMetadata.frame()/ImageOutputFrame.matrix() for D→O, and composes
the actual overview rational scale into physical image pixels. Target-normalized
coordinates are not PNG coordinates. Unframed64px minimum padding does not enlarge
the original source grid. There is no decode, new image/project/lease or mutation.

The strict document_region method maps all four corners only when the whole
positive ROI lies inside the saved source crop. No silent instruction clipping
or use of padded pixels as original content. For separately measured D shift d,
the second helper supplies A(B^-1(q)+d), supporting different crops, clockwise
turns, unequal output resizing and package scales without Kotlin frame math.

V can fill its agreed VerificationCoordinatePair DTO from these helpers plus
the checked package binding/source identity. Preserve content_size internally
when excluding padding from real registration. This is geometry only: it neither
measures alignment nor grants capture, source, verdict or mutation authority.
Keep the current framed-package refusal until V implements genuine normalized
paired-pixel comparison/registration and corresponding native failure checks.
Existing ordinary package compilation/output bytes and the six lineage fields
remain untouched. Preserve F81413d81's shared MCP JSON bounds when composing V.

Five unit regression methods are authored: independent crop/quarter-turn/scale
goldens; agreement with existing compiler mapping over all turns plus inverse;
translation across different output grids; minimum padding/equal-size turns;
bad extent/scale/source binding and partial-crop refusal. The actual compiled-PNG
image_output integration loop also checks this helper against its independent
marker pixel goldens. These new cases and whole package compilation are UNRUN:
the parent reserved the heavy slot for assigned model conversion; actual
conversion execution/artifacts have not been confirmed. Rustfmt
parse/format and whitespace checks passed; no synthetic fixture is reported as
real registration. Deferred command once the parent slot is free:

```bash
cargo test --locked --offline -p vw-package verification_geometry:: -j2
cargo test --locked --offline -p vw-package --test image_output -j2
```

R repair e212c275 is composed separately after supplied Astra acceptance. Its
three source/test leaves match exactly; nine independent production-helper cases
and corrected compiler probe are inherited source-scoped evidence. Transient
geometry movement currently uses a wireframe overlay; painted-pixel move preview
is not implemented. Whole Desktop/ABI/Compose/Windows/device validation remains
deferred. Android398c0f and AND4 source holds remain with its active owner.
