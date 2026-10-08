// Reaction Diffusion — Strobe base library, tier 3.
// Each sim pass advances Gray-Scott one step from the pass before it; the
// first reads the last one's previous frame, so the chain is four steps a
// frame with no pass ever reading itself. r = u (substrate), g = v (catalyst).

vec4 grayScott(sampler2D src) {
    vec2 px = 1.0 / RENDERSIZE;
    vec2 uv = _uv;
    vec2 c = texture(src, uv).rg;
    vec2 lap = -c;
    lap += 0.2 * (texture(src, uv + vec2(px.x, 0.0)).rg + texture(src, uv - vec2(px.x, 0.0)).rg
                + texture(src, uv + vec2(0.0, px.y)).rg + texture(src, uv - vec2(0.0, px.y)).rg);
    lap += 0.05 * (texture(src, uv + px).rg + texture(src, uv - px).rg
                 + texture(src, uv + vec2(px.x, -px.y)).rg + texture(src, uv - vec2(px.x, -px.y)).rg);

    float u = c.r;
    float v = c.g;
    float uvv = u * v * v;
    u += 1.0 * lap.r - uvv + feed * (1.0 - u);
    v += 0.5 * lap.g + uvv - (feed + kill) * v;

    // Seed: at start, on reseed, and wherever the kick lands this beat.
    vec2 spot = _hash22(vec2(floor(syn_BeatTime), 3.7));
    float d = length((uv - spot) * vec2(RENDERSIZE.x / RENDERSIZE.y, 1.0));
    float seed = step(d, 0.02) * step(0.5, syn_BassHits);
    if (FRAMECOUNT < 2.0 || reseed > 0.5) {
        u = 1.0;
        v = step(_hash12(floor(uv * 48.0)), 0.04);
    }
    v = max(v, seed);
    return vec4(clamp(u, 0.0, 1.0), clamp(v, 0.0, 1.0), 0.0, 1.0);
}

vec4 renderMain(void) {
    if (PASSINDEX == 0) { return grayScott(simD); }
    if (PASSINDEX == 1) { return grayScott(simA); }
    if (PASSINDEX == 2) { return grayScott(simB); }
    if (PASSINDEX == 3) { return grayScott(simC); }

    vec2 s = texture(simD, _uv).rg;
    float v = smoothstep(0.1, 0.35, s.g);
    float edge = length(vec2(dFdx(v), dFdy(v))) * 6.0;
    vec3 col = mix(vec3(0.02, 0.03, 0.06), ink, v);
    col += edge * vec3(1.0, 0.95, 0.85) * (0.4 + syn_HighLevel);
    return vec4(col, 1.0);
}
