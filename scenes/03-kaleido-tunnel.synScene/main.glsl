// Kaleido Tunnel — Strobe base library, tier 2.

vec4 renderMain(void) {
    vec2 p = _uvc - center;
    vec2 pol = _toPolarTrue(p);
    float r = pol.x;
    float a = pol.y + spin * 2.0 * PI;

    float seg = 2.0 * PI / segments;
    a = abs(mod(a, seg) - seg * 0.5);

    float depth = 0.35 / max(r, 0.001) + syn_BassTime * 0.6 + TIME * 0.4;
    vec2 tuv = vec2(a / seg * 4.0, depth);

    float checker = step(0.5, fract(tuv.x * 2.0)) * 2.0 - 1.0;
    checker *= step(0.5, fract(tuv.y)) * 2.0 - 1.0;
    float rings = _pulse(fract(depth * 0.5), 0.5, 0.15 + syn_BassHits * 0.3);

    vec3 col = _palette(depth * 0.1 + palette_step * 0.17,
        vec3(0.55), vec3(0.45), vec3(1.0), vec3(0.0, 0.33, 0.67));
    col *= 0.55 + 0.45 * checker;
    col += rings * vec3(1.0, 0.9, 0.7) * 0.6;
    col *= smoothstep(0.0, 0.35, r);
    return vec4(col, 1.0);
}
