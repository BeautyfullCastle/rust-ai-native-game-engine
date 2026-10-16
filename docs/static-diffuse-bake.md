# Bounded static sun diffuse bake

This is the first actual bake path for #101: deterministic CPU triangle rays
produce one diffuse bounce from static geometry under the existing directional
sun, stored in the existing SH9 irradiance grid and consumed by the unchanged
viewport shaders. Enable the existing opt-in `irradiance-probes` feature. It is
not general GI. Multiple bounces, sky lighting, points, emission, reflection
probes and specular IBL remain open follow-ups under #101.

## Transport convention

For each probe, weighted full-sphere sample directions find the nearest static
triangle. A miss contributes black. At a hit, the outgoing diffuse radiance is

`L = rho / pi * E_sun * max(dot(n, toward_sun), 0) * visibility`.

Visibility is a separate actual CPU triangle ray toward the sun. This catches
occluders which are not on the probe ray. The renderer's stylized sunlight uses
`E_sun = pi * sun_color * intensity`, so the factors of pi cancel at the bounce
surface. This matches the current direct diffuse convention; it is not a claim
of photometric units or a change to direct lighting. No hemisphere ambient,
previous probes, sky, point light, emissive or specular term enters the bake.

Incident radiance is projected onto the existing positive real SH9 basis
`[1,y,z,x,xy,yz,3z²−1,xz,x²−y²]`. The three bands are cosine-convolved once by
`pi`, `2*pi/3`, `pi/4`. The output is signed irradiance coefficients. The existing
GPU consumer interpolates them, evaluates at the receiver normal, clamps the
final evaluated irradiance nonnegative, and uses `albedo * E / pi`. No bake-time
energy clamp or second convolution is permitted. Invalid or out-of-range output
rejects the entire result.

## Scene and transaction boundary

Admission is local Yard3D Edit mode with no proposal preview. Only explicit
static bodies participate; zero velocity does not make a dynamic body static.
Dynamic, kinematic and animated geometry is excluded from both ray visibility
and bounce surfaces, but can receive the accepted grid at runtime. Imported
static bindings use actual model triangles rather than a second procedural
proxy. Geometry and materials come from a coherent read-only frame snapshot;
no host, ERP, simulation or immutable package content is changed by baking.

A separate runtime source-location identity owns file-verification stamps, so
retargeting identical content to another project root rechecks the current files.
The portable snapshot fingerprint uses canonical ordered content, including participating
world geometry, materials, verified model/texture identity, sunlight, grid
layout, algorithm/settings and epsilon. Camera, selection, exposure and movement
of excluded participants are not bake inputs. Included transform/material,
sunlight, grid and asset changes invalidate the receipt. Stale baked data is
visibly disabled. Rebake changed inputs, or restore the original inputs and
revalidate their hashes before reusing the receipt.

One background job is admitted at a time, with progress and cancellation. The
previous accepted grid remains in use while work runs. Completion rechecks
scene, mode, sidecar revision and participating inputs. Cancellation, stale
completion, resource failure and worker panic do not replace the grid or change
history, dirty state or files. Verified source file stamps guard completion and subsequent rendering. A reopened
baked sidecar stays disabled until asynchronous source verification succeeds.
A successful acceptance puts grid and receipt in
one sidecar undo entry. Saving remains a separate atomic file replacement; a
save failure preserves the in-memory result, history and dirty state.

## Sidecar compatibility

The grid coefficient format remains version 1. Existing authored/imported
sidecars remain version 1 and do not acquire receipts. A baked sidecar is
version 2, with grid provenance `baked` and a required version 1 bounded receipt.
The receipt records the algorithm version, SHA256 fingerprint, direction count,
triangle/participant/excluded counts, snapshot bytes, epsilon and bounded
triangle-test/traversal/time settings. Grid and
receipt are validated and saved together. Older binaries intentionally reject
the new sidecar version/provenance rather than silently treating baked data as
fresh authored light. Importing a coefficient grid explicitly produces imported
provenance, as before.

## Bounded work and limitations

The grid remains limited to 64 probes. Direction sampling defaults to 1024 and
is capped at 2048 per probe. Static geometry is capped at 8192 triangles.
Decoded textures are bounded to 32 MiB. The 64 MiB retained snapshot cap counts
owned buffer capacities, grid coefficients, strings, paths and metadata. It
excludes allocator bookkeeping, thread stacks and separately bounded temporary
extraction maps, BVH storage and verification stamps.
A bake permits at most 32 million triangle tests and 64 million BVH node visits,
with a 10-second default and 30-second absolute elapsed-time limit. Source asset
verification is bounded to 4,096 files and 256 MiB of streamed input per pass;
it never loads a second imported model on the UI thread. The bake uses a
deterministic BVH; independent
brute-force intersection is only a bounded test oracle. Budget exhaustion and
cancellation never yield partial accepted data.

SH9 remains a low-frequency representation with the existing one-cell boundary
fade; occlusion leakage and ringing are representation limitations. This work
does not add RHI formats, cubemaps, mips, reflection capture or a new render
pass. Software-GPU headless verification does not establish physical-GPU,
native-window or Windows-local behavior. Existing legacy `gpu3d` VIEW_FORMATS
failures and ignored tests remain separately reported, never a passing baseline.

## Verification commands

Use Rust 1.97.1, a functioning headless GPU adapter (software GL is acceptable),
`CARGO_INCREMENTAL=0`, and the existing locked dependencies. The focused gates
include:

```sh
cargo +1.97.1 test --release --locked --offline -j2 -p orr_render --features irradiance-probes,animation --lib --test irradiance_gpu
cargo +1.97.1 test --release --locked --offline -j2 -p orr_editor --features irradiance-probes,animated-models --lib --test yard3d_static_bake --test yard3d_irradiance
cargo +1.97.1 test --release --locked --offline -j2 -p orr_editor --features irradiance-probes --test yard3d_static_bake --test yard3d_irradiance
cargo +1.97.1 test --release --locked --offline -j2 -p orr_editor --lib --test deps
cargo +1.97.1 test --release --locked --offline -j2 -p orr_render --lib
cargo +1.97.1 clippy --release --locked --offline -j2 -p orr_editor --features irradiance-probes,animated-models --lib --test yard3d_static_bake --test yard3d_irradiance -- -D warnings
```

`STATIC_BAKE_CAPTURE_DIR` optionally records native viewport and composed editor
PNGs from the actual production host acceptance test. Assertions compare exact
host frame bytes, tick/checksum, scene files, history, sidecars and installed
package bytes. The test maintains identical viewport dimensions across reopen
before requiring byte-identical rendered output. Separate source-tamper tests
require immediate fallback, failed revalidation without endless retries, and
restored output after repaired sources are reopened.
