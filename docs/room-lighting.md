# Room lighting in the scene

A `room-escape-v1` project keeps its lighting in its scene file. The scene holds an environment, a sun and one point light as `orr_gfx` components, and the editor, the native player, captures and exported games all read them from the same Frame. There is no lighting document, no lighting panel and no `room-lighting` feature any more. The manifest entry has no `lighting` field, and a manifest that still names one is refused.

Lights and singletons are presentation data. No system reads them, so they change no simulation result. They are part of the Frame, so they are part of the Frame checksum, the snapshot and the replay start.

## What the scene holds

| Scene data | Role |
|---|---|
| `orr_gfx::Environment` (singleton) | Sky color, ground color and ambient strength. |
| `orr_gfx::PostProcess` (singleton) | The player draws its `exposure` and its tone mapping. The effect list is stored but the player does not draw it. |
| `orr_gfx::Sky` (singleton) | Stored for the scene window. The player does not draw sky, fog, clouds or wind. |
| one directional `orr_gfx::Light` with `orr_gfx::Transform3` | The sun: direction, color and intensity. |
| one point `orr_gfx::Light` with `orr_gfx::Transform3` | The point light: position, color, intensity and range. |
| `RoomEscapeV1::LightRule` on the point light | Whether the light follows the player and when it is on. |

The templates carry the three singletons with their default values. A scene file saved before these components existed still opens: a missing singleton gets its default value, and a Room without any light draws with the lighting every Room had before (the yard's low sun, the sky and ground ambient, no shadows).

A Room that has a light entity is lit only by its lights, as in every scene with `orr_gfx` lights. A scene with a point light and no sun has no sun. The conversion of the old documents therefore always adds the sun that was in force.

## Fields of the former lighting document

| Document field | Scene field |
|---|---|
| `point_light.position` | point `Light` entity, `Transform3.position` |
| `point_light.color` | `Light.color` |
| `point_light.intensity` | `Light.intensity` (scene range 0..100, the document allowed 0..1e4) |
| `point_light.range` | `Light.range` (scene range 0.01..1000, the document allowed up to 1e9) |
| `point_light: null` | no point light entity, or `Light.enabled` off |
| `follow_player` | `LightRule.follow` on, `LightRule.offset` (within 32 on each axis) |
| `activation` | `LightRule.activation`: `always`, `key_collected`, `escaped`, `key_missing`, `before_escape` |
| `environment.direction` | sun `Transform3.rotation` (pitch and yaw, degrees) |
| `environment.color` | sun `Light.color` |
| `environment.intensity` | sun `Light.intensity` |
| `environment.sky` | `Environment.sky_color` |
| `environment.ground` | `Environment.ground_color` |
| `environment.ambient` | `Environment.ambient` |
| environment preset file (`.room-environment-preset.json`) | the same six environment fields. The scene window's own presets and the saved scene replace the preset file. |

The sun direction `d` becomes pitch `asin(-d.y / |d|)` and yaw `atan2(d.x, d.z)`. A Room draws its sun without shadows, whatever the sun's shadow fields say. Fixed-point storage rounds colors and angles to about 0.00002. A converted Room differs from its former picture in a few hundred pixels by one level of one color channel.

## Editing

Open the Room in the editor and use the scene window with **Edit the open scene**. Its sun, ambient and exposure controls set the fields above. Each control is one undo step, and an undo restores the Frame checksum. The inspector edits every field of a light and of the rule.

To add a light, spawn one entity with `Transform3` and `Light` in one step, for example with `world.spawn`. A Room accepts at most one directional and one point light. An entity that is neither a room actor nor such a light is refused, and so is a light with any other component.

The former panel's viewport pick of a light's X/Z position is gone. Select the light in the hierarchy and edit its `Transform3`.

## Admission

The Room scene admission accepts exactly these entities: room actors with `Actor`, `Body` and `Collider`, and at most two light entities. A light entity holds `Transform3`, `Light` and, on a point light, optionally `LightRule`. It holds no other component. `Transform3.scale` is 1, positions and rotations are within the inspector ranges, light fields are within their inspector ranges, and `LightRule.offset` is zero unless `follow` is on. Environment and exposure values are range checked. The Room refuses everything it refused before.

## Player, capture and export

The native player and `room_escape --headless --capture` build their lighting from the frame they draw: the sun, ambient, exposure and tone mapping from `orr_gfx_view::scene_look`, and the point light from the first enabled point `Light`. With a rule, the light is on while its activation holds in `RoomRun`, and a following light is at the player's body position plus the offset, in world axes. The headless option `--check-model-poses` prints `room lighting active: 0/1` for a conditional light and `room carried light position: [...]` for a following light.

The export copies the scene, so the exported game has the lights. The export has no lighting document to check.

## Checksums

Registering `orr_gfx` adds components and singletons to the Room Frame, so every Room Frame checksum changed. The Room template's initial checksum is now `0xc6b08f4b43d8b9c9` (it was `0x0d36cae60075d641`). The `orr_games` golden checksums of other games did not change.

## Limits

This adds no shadows, light occlusion, flashlight cone or probes. The Room draws one sun and one point light. The imported-model renderer lights model receivers with the point light, and procedural proxies keep their existing shading. The native player and captures do not draw fog, sky, clouds, bloom or other post effects. The editor viewport draws the scene as it draws any `orr_gfx` scene. It was not compared with the player for these effects.
