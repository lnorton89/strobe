// Domain Warp — Strobe base library, tier 3.
// f(p + f(p + f(p))) after Inigo Quilez's domain-warping article, written
// from the description with the SSF noise library.
//
// The two inner layers (q and r, four fbm evaluations) are smooth warp
// fields, so pass 0 evaluates them at half the output size into "warpfield"
// and the output pass reads them back with bilinear filtering. Only the
// outermost fbm, which carries the visible fine detail, runs at full
// resolution. The fields are stored as q - 0.5 and r - 0.5: half-float
// resolution is finest near zero, and the warp multiplies any error by
// `warp`.

vec4 renderMain(void) {
    vec2 p = _uvc * zoom;
    float t = TIME * 0.08 + syn_Time * 0.05;

    if (PASSINDEX == 0) {
        vec2 q = vec2(_fbm(p + vec2(0.0, t), octaves),
                      _fbm(p + vec2(5.2, 1.3 - t), octaves));
        vec2 r = vec2(_fbm(p + warp * q + vec2(1.7, 9.2) + 0.15 * t, octaves),
                      _fbm(p + warp * q + vec2(8.3, 2.8) + 0.126 * t, octaves));
        return vec4(q - 0.5, r - 0.5);
    }

    vec4 w = texture(warpfield, _uv);
    vec2 q = w.xy + 0.5;
    vec2 r = w.zw + 0.5;
    float f = _fbm(p + warp * r, octaves);

    vec3 col = mix(vec3(0.10, 0.30, 0.40), vec3(0.65, 0.65, 0.50), clamp(f * f * 4.0, 0.0, 1.0));
    col = mix(col, vec3(0.0, 0.05, 0.16), clamp(length(q), 0.0, 1.0));
    col = mix(col, vec3(0.95, 0.55, 0.25) * (1.0 + syn_BassLevel), clamp(abs(r.x), 0.0, 1.0));
    col *= f * 1.8 + 0.2;
    return vec4(_gamma(col, 1.1), 1.0);
}
