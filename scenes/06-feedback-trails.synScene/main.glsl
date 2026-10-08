// Feedback Trails — Strobe base library, tier 2.
// Everything interesting here is the previous frame: each pixel reads
// syn_FinalPass at a slightly rotated, zoomed-in position and fades it.

vec4 renderMain(void) {
    vec2 p = _uvc;

    // Pull the last frame toward the centre with a twist.
    vec2 q = _rotate(p, twist * (0.5 + syn_MidLevel)) * (1.0 - zoom_in - syn_BassHits * 0.02);
    _uvc2uv(q);
    vec3 prev = texture(syn_FinalPass, q).rgb;
    prev = _hueRotate(prev, hue_drift) * decay;

    // A ring that blooms on every hit, and a wandering spark.
    float ring = abs(length(p) - 0.12 - syn_BassLevel * 0.25);
    float shape = smoothstep(0.012, 0.0, ring) * (0.3 + syn_BassHits);
    vec2 spark = vec2(sin(TIME * 1.3), cos(TIME * 1.7)) * 0.35;
    shape += smoothstep(0.03, 0.0, length(p - spark)) * (0.5 + syn_HighLevel);

    vec3 ink = _hsv2rgb(vec3(fract(TIME * 0.05 + syn_RandomOnBeat * 0.3), 0.85, 1.0));
    return vec4(max(prev, ink * shape), 1.0);
}
