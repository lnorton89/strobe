// Bloom Orbs — Strobe base library, tier 2.
//
// The lights are smooth 1/d^2 glows (the saturated core is about 14 pixels
// wide at 1080p), so the light pass renders at half size and the output pass
// reads it back with bilinear filtering. d^2 comes from a dot product, not a
// length squared.

vec4 blur(sampler2D src, vec2 dir) {
    // 9-tap Gaussian using linear filtering to read two texels per tap.
    vec2 px = dir / RENDERSIZE;
    vec4 sum = texture(src, _uv) * 0.2270270270;
    sum += (texture(src, _uv + px * 1.3846153846) + texture(src, _uv - px * 1.3846153846)) * 0.3162162162;
    sum += (texture(src, _uv + px * 3.2307692308) + texture(src, _uv - px * 3.2307692308)) * 0.0702702703;
    return sum;
}

vec4 renderMain(void) {
    if (PASSINDEX == 0) {
        vec2 p = _uvc;
        vec3 col = vec3(0.0);
        for (int i = 0; i < 32; i++) {
            float fi = float(i);
            if (fi >= count) { break; }
            float a = fi * 2.39996 + orbit * 6.2831 * (0.5 + fi / 32.0);
            float r = 0.08 + 0.38 * fract(fi * 0.618034);
            vec2 c = vec2(cos(a), sin(a)) * r * (1.0 + syn_BassLevel * 0.25);
            float size = 0.004 + 0.01 * _hash11(fi) + syn_BassHits * 0.006;
            vec2 v = p - c;
            vec3 hue = _hsv2rgb(vec3(fract(fi * 0.137 + TIME * 0.02), 0.75, 1.0));
            col += hue * size / max(dot(v, v) * 60.0, 0.0005);
        }
        return vec4(col, 1.0);
    }
    if (PASSINDEX == 1) { return blur(lights, vec2(1.0, 0.0)); }
    if (PASSINDEX == 2) { return blur(blurH, vec2(0.0, 1.0)); }

    vec3 base = texture(lights, _uv).rgb;
    vec3 glow = texture(blurV, _uv).rgb;
    vec3 col = base + glow * bloom * (1.0 + syn_Level);
    col = col / (1.0 + col);
    return vec4(_gamma(col, 1.2), 1.0);
}
