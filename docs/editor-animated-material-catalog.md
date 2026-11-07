# Inspect and select animated model materials

The cooked animated-model import screen can inspect a source before creating a color or PNG skin variant. The catalog shows material indices, which slots are used, primitive usage, linear color factors, image dimensions, sampler defaults and animation clips. It creates no files and grants no redistribution rights.

In an Editor built with `package-manager,room-animated-models`:

1. Open a Room project, expand **Import a model package**, select **Cooked animated model** and enter the `.animated.json` source.
2. Enable color or PNG skin creation, then click **Inspect animated source**. Wait for the read-only worker.
3. Review the source SHA-256, materials and clips. Click **Use material N** for a used material.
4. Complete the chosen variant inputs and the ordinary package/license fields, acknowledge distribution rights, then import.
5. Preview/add/verify, use the resulting asset, save bindings and reopen through the existing workflow.

Inspection also works in ordinary import mode, but material selection requires an active color or PNG operation. Only materials used by a primitive can be selected. Different materials can share the same image, while one material can affect several primitives; the usage count helps identify that scope. The color display describes stored linear factors, not rendered character appearance. Image dimensions and sampler defaults are data, not GPU previews.

Inspection requires the animation feature, a bounded regular cooked animated file and a fully valid animated payload with clips. Existing input path/link/stable-read checks are reused. There is no destination or license requirement for this read-only operation. Paths and project/source generations are captured with the worker. Changing source, mode or project retires old results, and inspection/import operations cannot overlap. Errors are shown without publishing or selecting a package.

A material chosen from the catalog carries the inspected original-source hash into the variant import worker. Import rereads and admits the source, then compares that snapshot before transformation or package staging. If the bytes changed at the same path, import refuses with no new package; inspect again before continuing. This checks the admitted source snapshot, not a filesystem lock or path pin through publication. A manual material-index edit clears the inspected selection guard; the existing manual route remains available.

Catalog inspection does not watch external file mutations. Its displayed information represents the admitted bytes when inspected. Original source hash and produced asset hash have different meanings: the former protects catalog selection, while **Asset SHA-256** and saved model bindings identify the new cooked payload. The existing producer embeds direct-input provenance.

Color and PNG creation remain mutually exclusive, with ordinary import as default. Existing license acknowledgment, source/PNG/license admission, explicit destination, no-overwrite staging and verified publication rules remain unchanged. Inspection never installs, assigns, changes the scene or automatically grants permission to use content. A stale or uncertain worker result must not be treated as a current imported package.

Focused acceptance covers catalog values, no-write behavior, feature gates, selected used slots, immutable worker identity, same-path source replacement refusal, actual widgets/import and package binding save/reopen. Native export checks deployed data and deterministic headless poses. CPU/UI-harness results do not establish actual GPU pixels, window/input/audio or play video. The locked computer remains locked and historical Windows publication failures are not retried.
