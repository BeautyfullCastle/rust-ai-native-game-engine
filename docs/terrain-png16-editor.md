# Replace terrain heights from a grayscale16 PNG

Enable the optional Editor feature `terrain-png16`. Open writable local terrain,
then expand **PNG16 height replacement**. Choose an absolute PNG path, a signed
height offset, a nonnegative height scale and optional Flip Z. Preview validates
and reports the replacement; **Apply PNG16 heights** is one undoable transaction.
Save remains a separate explicit action.

This workflow reads stored unsigned 16-bit grayscale sample codes. It does not
convert colors, apply gamma or profiles, rescale sBIT, resize images or infer
height ranges. PNG samples are big-endian as specified by the
[W3C PNG standard](https://www.w3.org/TR/png/#7Integers-and-byte-order).
The unchanged RAW16 integer formula is used:
`height_raw = offset_raw + floor(sample * scale_raw / 65535)`.
The PNG top row maps to the first terrain Z row unless Flip Z is selected.
There is no PNG endian selector. Existing RAW16 import and export retain their
separate controls and behavior.

The initial accepted subset is noninterlaced grayscale color type 0, bit depth
16. Dimensions must exactly match the current terrain. Alpha/transparency,
palettes, APNG, interlace, malformed CRCs, trailing bytes and unsupported chunks
are refused. Only image chunks and a small fixed-size metadata whitelist are
accepted; compressed text and color profiles cannot trigger metadata inflation.
Accepted gamma/sRGB/sBIT metadata never changes stored sample codes. The encoded
input is bounded to 1 MiB, chunk traversal and decoder allocation are bounded,
and the output size is exactly two bytes per sample before terrain conversion.

The converter retains the existing 2..129 dimension limit. Editor replacement
also requires the entire terrain footprint to be at most 4096 vertices, even
for a no-op. Height range, fixed-point overflow and exact display-coordinate
admission remain the existing terrain contract. No clamping or approximate
quantization is introduced.

Preview changes neither terrain nor saved files. Its candidate pins the original
encoded PNG bytes, input settings, current terrain content, source, scene and
load epoch. A failed Preview removes the previous candidate. Apply consumes the
candidate and rereads the PNG, so even metadata-only file changes are refused.
Use trusted, stable local paths; bounded file checks are not a sandbox against
hostile concurrent filesystem writers.

Only heights change. Terrain identity, X/Z layout, spacing and holes remain from
the current document; transparency is not a hole mask. Dirty terrain may be
previewed, and ordinary Undo/Redo remains available. Play, read-only/package
terrain and stale context block mutation. Source PNG bytes are never written.

Focused acceptance uses independent literal samples and RAW16 twins, malformed
and oversized input refusal, actual Preview/Apply/Undo/Redo/Save/reopen widgets,
and exact height/model data. This is CPU authoring-consumer evidence, not a new
renderer, physics, navigation, native-window or Windows acceptance claim.
