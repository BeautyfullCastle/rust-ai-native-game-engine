# Mobile targets: Android ARM64 and iOS

Status of M6 step 4. `docs/progress.md` and `docs/design-v1.md` are not edited here.

## Android (aarch64-linux-android)

**Built and tested here.** The sim crates (`orr_fp`, `orr_ecs`, `orr_sim`, `orr_proto`, `orr_session`,
`orr_testgame`, `orr_physics`, `orr_physics3d`) and `orr_games` (PhysGame, Yard3D; it has no wgpu, winit or
thread dependency, which is why they were split out of `orr_sample`) build for `aarch64-linux-android`:

```sh
rustup target add aarch64-linux-android
cargo build --release --target aarch64-linux-android \
  -p orr_fp -p orr_ecs -p orr_sim -p orr_proto -p orr_session -p orr_testgame -p orr_physics -p orr_physics3d -p orr_games --lib
```

Library builds need no NDK and no linker. Running the golden tests needs a linker, so the NDK is used and the
test binaries are run by `qemu-aarch64` (user mode emulation, `apt install qemu-user`):

```sh
NDK=/path/to/android-ndk-r27c/toolchains/llvm/prebuilt/linux-x86_64/bin
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android24-clang
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUNNER=qemu-aarch64
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C target-feature=+crt-static"
export CC_aarch64_linux_android=$NDK/aarch64-linux-android24-clang AR_aarch64_linux_android=$NDK/llvm-ar
cargo test --release --target aarch64-linux-android -p orr_fp --tests -- --test-threads=1
cargo test --release --target aarch64-linux-android -p orr_session --test arena -- golden --test-threads=1
cargo test --release --target aarch64-linux-android -p orr_physics --test physics -- golden --test-threads=1
cargo test --release --target aarch64-linux-android -p orr_physics3d --test golden -- golden --test-threads=1
cargo test --release --target aarch64-linux-android -p orr_games --test golden -- --test-threads=1
cargo test --release --target aarch64-linux-android -p orr_wasm_bench --test checksums -- --test-threads=1
```

`--test-threads=1` matters: with the default multi-threaded harness a static bionic binary under qemu-user
segfaulted in about one run in three (13 of 40 runs of `orr_ecs`'s `core` test), and never in 12 single-threaded
runs. It is the emulator's thread start-up, not the code under test.

`+crt-static` links bionic statically so qemu-user can run the binary without Android's dynamic linker; the
instructions executed are those of the `aarch64-linux-android` target. Doc tests are not cross-run. In this
session every golden test passed with the pinned values (orr_fp, orr_ecs, orr_session arena, orr_physics,
orr_physics3d, orr_games, orr_wasm_bench). The CI job `android-arm64` in `.github/workflows/determinism.yml`
does the same on `ubuntu-latest` (the image has the NDK in `ANDROID_NDK_LATEST_HOME`).

What qemu-user does **not** prove, and a real device run adds:

* Android's dynamic linker, libc behaviours and the app lifecycle (the sim crates are pure computation, so the
  risk is low).
* Speed. qemu is an emulator; tick times on a phone need a device. Expect the wasm-like profile of the 2D
  physics tick on a mid-range core, but this is a guess until measured.
* The view side: wgpu has a Vulkan/GLES Android backend, but `orr_sample` and `orr_editor` (winit window,
  egui) are not ported; winit needs the `android-activity` entry point (`cargo-apk` or `cargo-ndk` + a Gradle
  shell). Not attempted.

A device test, when one is available: `cargo ndk -t arm64-v8a test --release -p orr_games --no-run`
(or the manual linker setup above), `adb push` the test binaries and run them with `adb shell`; or run the
`orr_wasm_bench` `bench_runner` binary the same way to get ns per operation on the device.

`aarch64-unknown-linux-musl` under qemu also works without any NDK (`CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld`,
runner `qemu-aarch64`), and `orr_fp`'s golden passed there too; it is a quick aarch64 check on a machine without the NDK.

## iOS (aarch64-apple-ios, aarch64-apple-ios-sim)

**Not possible here** (needs macOS and Xcode). The steps:

1. `rustup target add aarch64-apple-ios aarch64-apple-ios-sim` on a Mac.
2. `cargo build --release --target aarch64-apple-ios -p orr_fp -p orr_ecs -p orr_sim -p orr_session -p orr_physics -p orr_physics3d -p orr_games --lib`:
   these crates use no OS API, so they should build as they do for Android. `orr_ffi` (the C ABI, `staticlib`) is the
   integration point: `cargo build -p orr_ffi --release --target aarch64-apple-ios` gives `liborr_ffi.a` for an Xcode
   project, with `crates/orr_ffi/include/orrery.h` as the header.
   `orr_ffi` pulls `orr_sample` (wgpu, winit, the relay client); if that does not build for iOS, split the physics client out
   the way `orr_games` was split.
3. Determinism: run the golden tests of the Android list on the **simulator** target
   (`aarch64-apple-ios-sim`, runs on an Apple Silicon Mac with `cargo test --target aarch64-apple-ios-sim`, or via
   `cargo-dinghy` for a device). The macOS ARM64 leg of the determinism CI (`macos-latest`, native aarch64) already
   runs the same CPU architecture and the same compiler back end, so iOS should agree; the simulator run is the proof.
4. Rendering: wgpu uses Metal; the 3D `Settings3D::LOW` preset (`physics3d --low`) is the starting point for phones.
5. Browser on iPhone: Safari has WebGPU since 26 and WebGL2; WebTransport needs a CA-signed certificate (no
   `serverCertificateHashes`, see `docs/webtransport-trial.md` section 7).

## Mobile-friendly view settings

`orr_render::Settings3D::LOW` and `physics3d --low` (`--mesh-segments N`, `--msaa N`, `--shadow-map N`):
no MSAA, 512 texel shadow map, 12-segment spheres and capsules. Cost reduction on lavapipe: render call 291 ms ->
55 ms per frame (`docs/wasm-bench.md`, section 6). The wasm build can add `WEB_SIMD=1` on phones' browsers (all
current mobile browsers have `simd128`).
