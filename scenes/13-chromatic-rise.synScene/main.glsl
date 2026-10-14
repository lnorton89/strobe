// Chromatic Rise: original Strobe interpretation of the owner's
// TouchDesigner video, https://youtu.be/w0JcYtmRr0U
//
// Pass 0 ("field", double-buffered) is a paint-drag feedback loop. Each
// frame it advects last frame's height and tint along a curl-noise flow,
// smears them along the flow (so streamlines turn into hair-like fibres),
// sharpens across the flow, then stamps in fresh fine noise and small
// bubbles. The final pass edge-detects and embosses that field, posterises
// it into hard neon hue steps and darkens the contours.
//
// Field channels: r = height (fine, bounded 0..1), g = tint (large islands),
// a = 1 once written (a cleared target reads 0, which re-seeds the field).

// Unsharp strength across the flow, and how many tint islands span 0..1.
const float SHARPEN = 0.25;
const float ISLANDS = 4.0;

// Flow in screen-height units: the curl of a two-octave noise potential
// plus a slow global drag that turns over time.
vec2 flowAt(vec2 p, float t) {
    vec2 q = p * flow_scale + vec2(t * 0.031, -t * 0.047);
    const float e = 0.06;
    float s0 = _fbm(q, 2.0);
    float sx = _fbm(q + vec2(e, 0.0), 2.0);
    float sy = _fbm(q + vec2(0.0, e), 2.0);
    vec2 curl = vec2(sy - s0, s0 - sx) / e;
    float a = 0.6 + 1.3 * sin(t * 0.045) + 0.5 * sin(t * 0.11 + 1.0);
    return curl + vec2(cos(a), sin(a)) * 0.45;
}

// Tint target: broad islands, contrast-stretched so they span the range.
float tintAt(vec2 p, float t) {
    vec2 q = p * 1.35 + vec2(t * 0.02, 3.1 - t * 0.015);
    float n = _fbm(q + 1.2 * vec2(_noise(q * 1.7 + t * 0.05), _noise(q * 1.7 - 4.0)), 3.0);
    return clamp((n - 0.5) * 3.2 + 0.5, 0.0, 1.0);
}

vec4 fieldPass(void) {
    float asp = RENDERSIZE.x / RENDERSIZE.y;
    vec2 px = 1.0 / RENDERSIZE;
    vec2 p = _uvc;
    float t = TIME * speed;
    float bass = clamp(syn_BassLevel, 0.0, 1.0) * reactivity;
    float hit = clamp(syn_BassHits, 0.0, 1.0) * reactivity;
    float high = clamp(syn_HighLevel, 0.0, 1.0) * reactivity;

    vec4 prev = texture(field, _uv);
    if (prev.a < 0.5) {
        // First frame, or the target was resized: seed with developed noise.
        float h0 = _fbm(p * 9.0 + 5.0, 3.0);
        return vec4(clamp((h0 - 0.5) * 2.5 + 0.5, 0.0, 1.0), tintAt(p, t), 0.0, 1.0);
    }

    // Broad islands decide colour, texture roughness and smear length.
    float tint = tintAt(p, t);
    float rough = smoothstep(0.3, 0.7, tint);

    // Advect: sample upstream along the flow.
    vec2 vel = flowAt(p, t) * flow * 0.0032 * (1.0 + 1.6 * bass + 0.9 * hit);
    vec2 uvS = _uv - vel / vec2(asp, 1.0);
    // Direction fades to nothing where the flow stalls, so no hatching there.
    vec2 vT = vel * RENDERSIZE;
    vec2 dirT = vT / max(length(vT), 0.7);
    vec2 stepT = dirT * px;
    vec2 perpT = vec2(-dirT.y, dirT.x) * px;
    float len = smear * (0.45 + 0.9 * rough) * (1.0 + 1.5 * bass);

    vec4 a = texture(field, uvS);
    vec4 b = texture(field, uvS - stepT * len);
    vec4 c = texture(field, uvS + stepT * len);
    vec4 l = texture(field, uvS - perpT);
    vec4 r = texture(field, uvS + perpT);

    float h = 0.5 * a.r + 0.25 * (b.r + c.r);
    float g = 0.5 * a.g + 0.25 * (b.g + c.g);
    // Unsharp across the flow keeps the fibres crisp against the smear.
    h += SHARPEN * clamp(a.r - 0.5 * (l.r + r.r), -0.12, 0.12);

    // Fresh striations: fine isotropic noise. The smear along the flow
    // (below, next frame) stretches it into fibres that follow the
    // streamlines, so no fibre frame has to be derived from the flow.
    float fibHi = fibers * (1.0 + 0.6 * high);
    float fib = _noise(p * fibHi + vec2(t * 0.15, 3.0));
    h += (fib - 0.5) * (0.03 + 0.10 * rough + 0.05 * high);
    g += (fib - 0.5) * 0.03;
    // Large islands of height, so the fine fibres ride on broad shapes.
    h = mix(h, _fbm(p * 2.6 + vec2(t * 0.03, 1.7), 2.0) * 1.3 - 0.15, 0.03);

    // Bubbles: one disc per grid cell, born once per cycle, clustered by a
    // slow region mask, then dragged away by the flow like everything else.
    const float cells = 22.0;
    vec2 bp = p * cells + vec2(0.0, 7.0);
    vec2 cell = floor(bp);
    vec2 f = fract(bp) - 0.5;
    vec4 rn = _hash42(cell);
    float period = 32.0;
    float ph = FRAMECOUNT + rn.z * period;
    float cyc = floor(ph / period);
    float born = step(mod(ph, period), 1.0);
    float roll = _hash12(cell + cyc * 17.3);
    float region = smoothstep(0.25, 0.55, tint);
    float want = born * step(roll, bubbles * (0.15 + 0.85 * region));
    float rad = 0.28 + 0.2 * rn.w;
    float disc = want * (1.0 - smoothstep(rad - 0.04, rad, length(f - (rn.xy - 0.5) * 0.35)));
    float core = smoothstep(rad * 0.35, rad * 0.75, length(f - (rn.xy - 0.5) * 0.35));
    h = mix(h, fract(roll * 7.0 + cyc * 0.37 + 0.3 * core), disc);
    g = mix(g, fract(roll * 5.0 + 0.3), disc * 0.6);

    // Tint follows slowly moving islands.
    g = mix(g, tint, 0.012 + 0.01 * hit);

    // Folding instead of clamping keeps fine detail alive at the extremes.
    return vec4(_mirror(h), clamp(g, 0.0, 1.0), 0.0, 1.0);
}

vec4 postPass(void) {
    vec2 uv = _uv;
    vec2 px = 1.0 / RENDERSIZE;
    float bass = clamp(syn_BassLevel, 0.0, 1.0) * reactivity;
    float high = clamp(syn_HighLevel, 0.0, 1.0) * reactivity;

    vec2 c = texture(field, uv).rg;
    float hl = texture(field, uv - vec2(px.x, 0.0)).r;
    float hr = texture(field, uv + vec2(px.x, 0.0)).r;
    float hd = texture(field, uv - vec2(0.0, px.y)).r;
    float hu = texture(field, uv + vec2(0.0, px.y)).r;
    vec2 grad = vec2(hr - hl, hu - hd);
    float edge = length(grad);

    // Posterise: tint picks the island, height steps within it.
    float v = c.g * ISLANDS + c.r * bands;
    float idx = floor(v);
    float fr = fract(v);
    float phase = hue + TIME * color_speed * 0.02 + syn_BassTime * 0.19 * reactivity;
    // Islands take far-apart hues; the height steps inside one jump a fixed
    // 70 degrees, so accents sit hard against the island's own colour.
    float hh = fract(floor(c.g * ISLANDS) * 0.381966 + floor(c.r * bands) * 0.2 + phase);
    float sat = 0.78 + 0.22 * fract(idx * 0.7071);
    float val = 0.84 + 0.16 * fract(idx * 0.3141 + 0.2);
    vec3 col = _hsv2rgb(vec3(hh, sat, val * (0.8 + 0.3 * fr)));

    // Relief: lit from the upper left, bright pastel on the lit slope.
    float lit = dot(grad, normalize(vec2(-0.6, 0.8))) * relief * 9.0;
    col *= 1.0 + 0.8 * clamp(lit, -0.8, 0.8);
    col = mix(col, vec3(1.0), clamp(lit - 0.4, 0.0, 1.0) * 0.4);

    // Contours: where a hue step lands, and where the field changes fast.
    float w = fwidth(v) * 1.8 + 0.03;
    float step_line = 1.0 - smoothstep(0.0, w, min(fr, 1.0 - fr));
    float ink = max(step_line, smoothstep(0.10, 0.28, edge));
    ink *= 1.0 - 0.6 * smoothstep(0.5, 1.4, fwidth(v));
    col = mix(col, col * 0.08, ink * outline);

    // Grit.
    float n = _hash12(floor(uv * RENDERSIZE) + fract(TIME) * 61.0);
    col *= 1.0 + (n - 0.5) * (0.10 + 0.25 * high);
    return vec4(clamp(col, 0.0, 1.0), 1.0);
}

vec4 renderMain(void) {
    if (PASSINDEX == 0) { return fieldPass(); }
    return postPass();
}
