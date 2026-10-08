// Aurora Gradient — Strobe base library, tier 1.
// A handful of sines per pixel: the floor every other scene is measured from.

vec4 renderMain(void) {
    vec2 p = _uvc;
    float t = TIME * drift * 0.2;

    float bands = sin(p.x * 2.1 + t) * 0.5 + sin(p.x * 3.7 - t * 1.3) * 0.25;
    float y = p.y + bands * 0.18;

    vec3 col = _palette(y * 0.8 + t * 0.15 + hue_shift,
        vec3(0.5), vec3(0.5), vec3(1.0, 0.9, 0.8), vec3(0.0, 0.15, 0.35));
    col *= smoothstep(0.75, -0.2, abs(y));

    float horizon = exp(-abs(y + 0.25) * (14.0 - 8.0 * syn_BassLevel));
    col += vec3(1.0, 0.55, 0.25) * horizon * glow * (0.35 + syn_BassLevel);

    return vec4(col, 1.0);
}
