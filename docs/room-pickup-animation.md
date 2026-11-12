# Room key-pickup animation

In Room character authoring, enable **Play key pickup clip once**, select an admitted animation clip and choose its playback speed. Apply the character draft, then use the existing paired Save to persist both model binding and character settings. Undo/Redo and reopening use the existing authoring path.

The optional schema-6 policy lives inside `motion.pickup`. Existing schema 1–5 documents omit it and retain their original behavior. Pickup requires the existing motion/rates/crossfade policy; idle policy remains enabled while pickup is active. Other clip, speed, crossfade and escaped-completion controls remain editable. Disabling pickup explicitly keeps the motion settings and returns to schema 5.

The shared Editor/native presentation cursor triggers only after observing consecutive simulation ticks transition from Searching to Carrying. First observing a character already carrying a key, or skipping over an unknown transition, does not invent a pickup event. Once triggered, later forward observation gaps preserve its known start time. Escaped takes priority. Rest-pose preview, document/model/context/entity changes and rewind reset transient playback.

Pickup plays over sampled times before the clip duration. At the duration boundary it returns to the current Carrying idle/moving clip at its normal absolute phase; there is no extra endpoint-hold tick. Zero-duration clips immediately fall back. The authored crossfade also applies when pickup and Carrying use the same clip. Presentation never modifies the gameplay Frame or save identity.

Focused CPU acceptance covers strict codec/legacy bytes, clip admission, rates and transition boundaries, reset/error atomicity, actual UI paired persistence and source-hidden headless native playback. For schema 6, `--check-model-poses` additionally reports a SHA-256 digest of the actual observed character pose. This diagnostic is a comparison receipt, not publisher authentication. Existing schemas keep their diagnostic output.

Windows execution, GPU pixels, a physical game window and playback video remain unverified unless separately recorded.
