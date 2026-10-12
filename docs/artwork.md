# Generated artwork

## Logo and animated logo (2026-10-12)

[`frontend/assets/logo.png`](../frontend/assets/logo.png) pairs the existing lavender companion with the exact title **INVISIBLE AI**. It is a 1536 × 1024 transparent PNG generated with the built-in image generation tool using `everyday-companion.png` as the character reference. The initial and final refinement prompts are saved in [`logo.prompt.txt`](../frontend/assets/logo.prompt.txt).

[`frontend/assets/logo.gif`](../frontend/assets/logo.gif) animates that artwork with a gentle floating motion and a small tilt while the title stays still. It is a transparent 768 × 512 GIF with 72 frames in a seamless 3.6-second loop. GIF supports only binary transparency; the PNG retains the original alpha edges. The standalone [`scripts/animate-logo.mjs`](../scripts/animate-logo.mjs) exporter requires Sharp, either installed locally or supplied through `SHARP_MODULE`, and does not add an app runtime dependency. The GIF is an optional shareable asset; app surfaces continue using their existing motion-aware companion.

## Companion hero (2026-10-12)

[`frontend/assets/invisible-ai-hero.png`](../frontend/assets/invisible-ai-hero.png) features Invisible AI's main character floating above an open notebook, with notes and a photo orbiting around it. The existing [`everyday-companion.png`](../frontend/assets/everyday-companion.png) supplied the character reference, preserving its lavender glass body, dark violet eyes, smile and waving arm.

Generated with the built-in image generation tool and copied into the repository at the existing 1536 × 1024 hero path. The README, guide and Assistance header reuse this asset. The full final prompt is saved in [`invisible-ai-hero.prompt.txt`](../frontend/assets/invisible-ai-hero.prompt.txt).

## Harbour

Asset: [`frontend/assets/harbour.png`](../frontend/assets/harbour.png), used in the web interface's capture panel. Generated with the built-in `image_gen` tool through the imagegen skill, then copied into the repository. No API fallback was used.

Final prompt:

> Use case: stylized-concept
> Asset type: wide decorative hero artwork for Invisible AI (then named Causewaybay Bot), a polished personal screen-to-answer workspace.
> Primary request: An elegant miniature architectural sculpture of Hong Kong Causeway Bay and Victoria Harbour, formed from translucent lilac glass and matte ivory ceramic, on a soft warm ivory studio background. A few tall slender towers, a tiny circular violet sun and sweeping glass harbour curve. Editorial 3D still life, refined and quiet, tactile surfaces, gentle shadows, soft lilac reflections. Wide landscape composition, buildings concentrated in the right two thirds with negative space on left. No text, no letters, no logos, no interface, no watermark. Sophisticated restrained palette of off-white, pale lavender and deep violet.


## Everyday companion (2026-10-11)

`frontend/assets/everyday-companion.png` was generated with the built-in imagegen tool, with a transparent background, for the empty chat screen. Motion's locally bundled JavaScript API supplies a brief spring greeting and interaction transitions; Reduce Motion disables them.

Prompt: “Use case: stylized-concept. Asset type: small empty-chat companion illustration for an everyday personal AI app, Invisible AI. Primary request: a friendly, playful little translucent rounded spirit with two expressive dot eyes, a subtle smile and a small waving arm, like a soft glass-and-jelly desk companion. Simple memorable silhouette, delightful and approachable for daily use, polished soft 3D illustration, gentle lavender tint fitting the app's existing purple accents. One character centered with comfortable padding, readable at 120 pixels. Soft studio lighting. No text, no logo, no UI, no scenery, no extra objects. Transparent background.”

The companion now anchors the main chat interface: the original transparent illustration is layered over a Three.js orbit and small illuminated satellites. Motion supplies floating, listening, thinking and touch/keyboard greeting reactions. It stays compact during a conversation. Existing waiting indicators reuse the character while retaining real status text and progress. Animations pause offscreen or in a background tab, respect Reduce Motion, and keep the original image usable without WebGL. All code and assets are bundled locally.
