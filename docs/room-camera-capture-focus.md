# Room viewport camera capture

Use the existing Capture Room viewport camera controls to Stage the current view, Apply it in memory, and Save explicitly. Undo/Redo remain available for the captured camera. Perspective FOV and follow/bounds policies are preserved; orthographic zoom is rebased into the saved half-height so the composition remains stable.

Stage, Apply, Undo, Redo and Save require a focused window. A focus-loss event in the current input frame blocks these actions even if a focus-gain event follows in the same frame. Cancel and Finish remain available for clearing transient state. Existing clean local Room, source coherence and idle-work guards remain in effect.

CPU regressions exercise queued accessibility actions and focus changes in the same frame, normal capture/history/save behavior, projection parity, context invalidation and saved-document reopen. These do not establish physical window playback, GPU output or Windows execution.
