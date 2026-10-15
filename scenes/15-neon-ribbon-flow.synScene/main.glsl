// Neon Ribbon Flow: original procedural interpretation of the owner's Lethal
// Larry-style video. Broad translucent neon ribbons that twist and cross in
// depth, with iridescent cross-sections, glossy crests and soft bloom.
//
// Two passes: a one-texel clock integrates the flow speed (so mids can make
// it surge without the pattern jumping), then the ribbons are drawn
// analytically, back to front. Every edge is a smoothstep over the pixel
// footprint, and the fine filaments fade out when they would be thinner than
// a couple of pixels, so there is no moire at any size.

float flowRate() {
    return speed * (1.0 + 0.9 * clamp(syn_MidLevel, 0.0, 1.0) * reactivity);
}

float stepDt(float prevTime) {
    float d = TIME - prevTime;
    return (d > 0.0 && d < 0.2) ? d : 1.0 / 60.0;
}

vec4 clockPass() {
    vec4 prev = texelFetch(clock, ivec2(0), 0);
    // A cleared target (first frame, or a resize) restarts at the scene time.
    if (prev.x <= 0.0) { return vec4(TIME, TIME * flowRate(), 0.0, 1.0); }
    float dt = stepDt(prev.x);
    return vec4(TIME, prev.y + dt * flowRate(), 0.0, 1.0);
}

// Iridescent neon: a full wheel, stretched toward magenta and cyan.
vec3 neon(float h) {
    vec3 c = clamp(0.5 + 0.62 * cos(6.2831853 * (h + vec3(0.0, 0.33, 0.67))), 0.0, 1.0);
    return c * c * (3.0 - 2.0 * c);
}

vec4 ribbons(void) {
    float T = texelFetch(clock, ivec2(0), 0).y;
    float bass = clamp(syn_BassLevel, 0.0, 1.0) * reactivity;
    float high = clamp(syn_HighLevel, 0.0, 1.0) * reactivity;
    float kick = clamp(syn_BassHits, 0.0, 1.0) * reactivity;

    vec2 p = _uvc / zoom;
    p *= 1.0 - 0.04 * bass;
    float px = 1.0 / (RENDERSIZE.y * zoom);

    vec3 col = vec3(0.0);
    for (int i = 0; i < 5; i++) {
        float fi = float(i);
        float z = fi / 4.0;                       // 0 far .. 1 near
        float r1 = _hash11(fi * 3.17 + 1.3);
        float r2 = _hash11(fi * 5.31 + 7.9);
        float r3 = _hash11(fi * 2.73 + 3.1);
        float r4 = _hash11(fi * 4.11 + 9.7);
        float r5 = _hash11(fi * 6.43 + 2.2);

        // Liquid bend: a slow swirl of the plane, different for every ribbon.
        vec2 pw = p + curl * (0.055 + 0.04 * z) * vec2(sin(p.y * 3.1 + T * 0.19 + fi * 1.7),
                                                      sin(p.x * 2.7 - T * 0.15 + fi * 2.3));
        float ang = (r1 - 0.5) * 1.5 + 0.30;
        vec2 dir = vec2(cos(ang), sin(ang));
        vec2 nrm = vec2(-dir.y, dir.x);
        // Parallax: nearer layers drift faster against the far ones.
        float x = dot(pw, dir) + T * (0.020 + 0.060 * z);
        float y = dot(pw, nrm) + (r2 - 0.5) * 0.85;

        float k = mix(5.6, 3.4, z) * (0.8 + 0.4 * r3);
        float amp = mix(0.09, 0.22, z) * (0.7 + 0.6 * r4);
        float hw = mix(0.024, 0.100, z) * (0.75 + 0.5 * r5) * width * (1.0 + 0.22 * bass);

        float ph = x * k + fi * 2.1 - T * (0.30 + 0.20 * r3);
        float ph2 = ph * 2.13 + 1.3 + T * 0.12;
        float c = amp * (sin(ph) + 0.45 * sin(ph2));
        float dc = amp * k * (cos(ph) + 0.45 * 2.13 * cos(ph2));
        float dist = (y - c) / sqrt(1.0 + dc * dc);

        // The ribbon twists: its apparent width follows the facing angle.
        float facing = cos(x * k * 0.33 + fi * 1.3 + T * 0.14);
        float tw = sqrt(facing * facing + 0.05);
        float h = hw * (0.30 + 0.70 * tw);
        float u = clamp(dist / h, -1.0, 1.0);
        float gw = mix(0.035, 0.022, z) * (1.0 + 0.5 * bass);
        float hue0 = hue + fi * 0.19 + 0.10 * sin(T * 0.07 + fi) + 0.07 * kick;
        // Far from the ribbon and its glow: nothing to draw.
        float edge = abs(dist) - h;
        if (edge > 5.0 * gw) { continue; }

        // Far layers are softer (depth of field), near ones crisp.
        float aa = px * mix(4.5, 1.1, z);
        float cover = 1.0 - smoothstep(h - aa, h + aa, abs(dist));

        float nz = sqrt(max(1.0 - u * u, 0.0));
        float side = clamp(facing * 3.0, -1.0, 1.0);
        float cu = (u - 0.42 * side) * 5.0;
        float crest = exp(-cu * cu);
        // Marbling inside the ribbon: the colour bands wander as they flow.
        float mar = sin(u * 2.2 + x * k * 0.9 + 1.7 * sin(x * k * 0.4 + T * 0.3) + fi);
        float hh = hue0 + hue_spread * (0.55 * u + 0.12 * sin(x * k * 0.6 + fi) + 0.30 * mar);

        vec3 base = neon(hh);
        // Fine filaments along the ribbon; faded when finer than ~3 px.
        float period = h * 2.0 / max(filaments, 1.0);
        float fade = clamp(period / (px * 6.0) - 0.4, 0.0, 1.0);
        float fil = 0.5 + 0.5 * cos(u * 3.14159265 * filaments + 1.6 * sin(x * k * 0.5 + fi) + 0.8 * mar);
        base *= mix(1.0, 0.55 + 0.75 * fil, 0.6 * fade);
        // Second hue on the crest: thin-film shift toward the neighbour colour.
        base = mix(base, neon(hh + 0.18), 0.5 * smoothstep(0.2, 1.0, abs(u)));

        float depth = mix(1.0 - depth_fog, 1.0, z);
        vec3 rib = base * (0.38 + 0.62 * nz) * (0.65 + 0.35 * tw);
        float ru = (u + 0.62 * side) * 8.333;
        float refl2 = exp(-ru * ru);
        vec3 hot = mix(vec3(1.0, 0.97, 0.92), base, 0.30);
        rib += hot * (crest + 0.35 * refl2) * gloss * (0.4 + 0.6 * tw) * nz;
        float rimk = 1.0 - nz;
        rib += base * (rimk * rimk * rimk) * 0.55;       // neon edge
        rib *= depth;

        // Sparkle on the crest: soft dots that twinkle with the highs.
        if (crest > 0.05 && cover > 0.0) {
            vec2 sg = vec2(x * 70.0, u * 5.0 + fi * 3.0);
            vec2 cid = floor(sg);
            float dotAt = _hash12(cid + floor(T * 3.0 + fi));
            vec2 jit = _hash22(cid) * 0.5 + 0.25;
            float dsp = length(fract(sg) - jit);
            float sparkle = (1.0 - smoothstep(0.0, 0.30, dsp)) * step(0.82 - 0.35 * high, dotAt) * crest;
            rib += vec3(1.0) * sparkle * (0.25 + 1.6 * high) * depth;
        }

        // Translucent over what is behind; glow screens on top.
        // A faint contact shadow where a nearer ribbon passes over what is behind.
        float shade = (1.0 - cover) * exp(-max(edge, 0.0) / (0.010 + 0.01 * z));
        col *= 1.0 - 0.40 * shade * z;
        col = col * (1.0 - cover * 0.60) + rib * cover;
        float halo = exp(-max(edge, 0.0) / gw) * (1.0 - cover);
        col += neon(hue0 + 0.06) * (halo * 0.20 + crest * 0.06) * glow * depth;
    }
    // Keep saturation when ribbons overlap: soft shoulder, black stays black.
    col = 1.0 - exp(-1.5 * col);
    return vec4(col * brightness, 1.0);
}

vec4 renderMain(void) {
    if (PASSINDEX == 0) { return clockPass(); }
    return ribbons();
}
