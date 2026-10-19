# Pre-schema-3 schema-2 golden bytes

Captured with the untouched producer at commit
`177ff1ca1397e9ba23405e15c673843e767c680a`, before named-clip implementation.
The source `authoring.rs` SHA-256 was
`2de6115d1558709080c965857595a306a69473abfb8110d51cf2bede3a6441d4`.

The input is synthetic original RGBA artwork with unequal 2×2, 1×3 and 1×1
images, explicit offsets in a 3×3 canvas, two columns, and a Loop clip with
30/70/20 ms frames. `schema2/input` contains the exact original inputs;
`schema2/output` contains all four original generated files. The CPU CLI
contract compares every output byte and pins each original SHA-256 literal.
The local `.gitattributes` disables text conversion so CRLF license bytes
and JSON/newline bytes remain unmodified on every checkout.

The schema-1 regression reuses the existing independently frozen pre-canvas
fixtures without modifying them. These are compatibility oracles, not output
regenerated from the implementation being tested.
