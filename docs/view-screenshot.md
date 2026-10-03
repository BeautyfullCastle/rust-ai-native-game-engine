# ERP view.screenshot contract (issue #19)

Status: draft for routing review. Base: `68ff783a0fa71698c0e75fa26997a458352a8643`. Implementation starts after independent review. This API captures the owning editor's app framebuffer, including its panels and current primary scene view.

## Owner and route

An editor started on a local host installs one typed in-process screenshot endpoint in `ServerConfig` before that host starts. A request received on that host's ERP listener or local connector travels through one bounded mailbox to its `EditorApp`. The app continues to pump ERP and draw while the request is pending. No host renderer, filesystem path, desktop capture or network provider-registration method is added.

A headless host has no endpoint and returns `view_unavailable`. An editor attached to an independently running remote host does not install an endpoint on that host: merely subscribing to its frames does not establish a trusted screenshot provider. This first version therefore returns `view_unavailable` on that remote ERP endpoint even when such an editor window is open. Cross-process editor routing is follow-up work and is not advertised as supported.

## Public ERP surface

`view.screenshot` requires existing `read` capability, checked before availability or resource admission. JSON-RPC requests require an ID; notifications do not allocate capture work. Params are an object with optional `target` (only `"app_framebuffer"`), `timeout_ms` (integer 50..5000, default 5000), `max_width` and `max_height` (integers 1..2048, defaults 2048). Unsupported keys, camera selectors, output paths and target types are invalid params. Capture always waits for paused, settled presentation; it does not pause the host or change the scene.

A successful response contains `status:"captured"`, `source:"editor.app_framebuffer"`, `game`, `build_id`, `mode`, `paused:true`, decimal-string `tick`, hex-string `checksum`, decimal-string `frame_seq` and `ui_frame`, integer `width`/`height`, `mime_type:"image/png"` and base64 `png_base64`. It describes an image produced for this request, never a cached image. `rpc.discover` lists the method and its capability/limits. Direct `call_local` embedding without the server/mailbox is outside this asynchronous transport method's support.

## Frame affinity and stale presentation

The server remembers the requested authoritative view key, checksum and host view incarnation. The editor waits for its ordinary paused-frame/model refresh readiness, expired agent pulses, no active gesture and no proposal preview. The primary snapshot must match the requested state before capture. Unsupported proposal previews return explicit unavailable rather than attributing their pixels to the primary snapshot.

At the UI pass that renders and requests the image, the app stores that pass's immutable snapshot stamp and frame counter, and uses a unique typed `egui::UserData` ticket on `ViewportCommand::Screenshot`. Only the matching `Event::Screenshot` completes that ticket. Response metadata comes from the stored capture pass, not from a later pump or screenshot event's arrival time.

If the primary frame, host incarnation, scene/session state or capture dimensions change while pending, return `view_stale` and discard the image. Old, duplicate or foreign screenshot events cannot complete another request. Width/height are taken from the returned image and must match the planned framebuffer dimensions. A success never labels earlier pixels as a current later tick/checksum.

## Resource and completion bounds

Only one pending screenshot is admitted per owning endpoint/host, with no request queue or retained image cache. Another request ID gets `view_busy`. Repeating the same connection/JSON-RPC ID while it is pending coalesces with that request and emits at most its original terminal response; it does not start another capture. The request must finish by its total wall-clock deadline, whether waiting for settling or GPU delivery.

Each dimension is at most 2048 and the total is at most 1,048,576 pixels. RGBA staging is at most 4 MiB; PNG encoding uses a writer that rejects output exceeding 4 MiB. Base64 plus bounded metadata is below 6 MiB per result. Reject oversized framebuffer dimensions before requesting capture, and recheck the actual returned image before flattening pixels/encoding. Encoding and capture never write files. There is no stored result after the host sends or discards it. These bounds cover this capture work and one result; they are not a total RSS or downstream ERP/socket-queue bound.

Owner drop/restart/host disconnect produces `view_unavailable` for still-connected callers and releases its reservation. Requester disconnect cancels its ticket and discards any late completion without sending a response to a new connection. Timeout returns `view_timeout`, cancels the ticket and discards late GPU results. A capture/encode failure returns `view_capture_failed`; capability failure remains `permission_denied`; invalid params remain `invalid_params`. Success and each failure release pending capacity once. Host polling checks completion/deadlines without waiting for the UI or GPU; idle-host waits are bounded while a capture is active.

## Files and validation

Handed-off files: editor `src/app.rs`, `src/editor.rs`, `src/backend.rs`; remote `src/methods.rs`, `src/server.rs`; screenshot-focused tests. Proposed small additions: new remote `src/screenshot.rs` typed mailbox/budget helpers, one `src/lib.rs` public-module export, new remote `tests/screenshot.rs`, editor screenshot-focused tests, and this document. No `link/net/local/host/sample/dispatch` changes, renderer dependencies, Cargo.lock, asset/cooker, event tests, shared workflows, progress or roadmap changes are planned. Any additional shared file needs a separate scope handoff first.

Meaningful tests cover permission-first denial, missing/dropped owner, response correlation, one-request admission/duplicate ID, timeout/requester disconnect/late result cleanup, dimensions/pixels/PNG-output limits, unchanged UI pumping, paused settling, proposal/stale/resize failure and exact PNG-to-capture-stamp association. Existing screenshot-settle, Arena native smoke, golden/assertion and mandatory CI checks remain. A synthetic egui screenshot event proves routing and metadata handling, not actual GPU readback. The existing Linux Xvfb/native-window path should additionally exercise actual eframe framebuffer capture when available; Windows or physical-device claims require separate execution evidence.

Local execution remains serialized with Haneul #28. Validation will record exact source head, commands, raw outcomes and unverified platform scope. A draft PR and its exact-head full CI follow implementation and independent source review. No merge is authorized.

