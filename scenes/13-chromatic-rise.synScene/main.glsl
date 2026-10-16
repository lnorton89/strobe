// Chromatic Rise: original Strobe interpretation of the owner's
// TouchDesigner video, https://youtu.be/w0JcYtmRr0U
//
// Pass 0 ("clock", one texel) integrates the speed controls, so moving a
// slider never jumps the pattern, and gives the frame delta.
//
// Pass 1 ("flowfield", quarter size) evaluates the three broad noise fields
// the loop steers by (the flow, the tint islands and the large height
// islands). They change over seconds and have features hundreds of pixels
// wide, so bilinear filtering of a quarter-size copy is indistinguishable
// and the full-size pass pays for one noise lookup instead of eleven.
//
// Pass 2 ("field", double-buffered) is a paint-drag feedback loop. Each
// frame it advects last frame's height and tint along a flow made of long
// shear lanes (the drags) and a curl-noise swirl, smears them along the flow
// (so streamlines turn into hair-like fibres), sharpens across the flow,
// then stamps in fresh fine noise and clusters of small bubbles. The final
// pass edge-detects and embosses that field, posterises it into hard acid
// colour steps and darkens the contours.
//
// Flowfield channels: rg = flow, b = tint target, a = large height islands.
// Field channels: r = height (fine, bounded 0..1), g = tint (large islands),
// a = 1 once written (a cleared target reads 0, which re-seeds the field).

// Unsharp strength across the flow, and how many tint islands span 0..1.
const float SHARPEN = 0.4;
const float ISLANDS = 4.0;

float stepDt(float prevTime) {
    float d = TIME - prevTime;
    return (d > 0.0 && d < 0.2) ? d : 1.0 / 60.0;
}

// x = TIME, y = flow time (speed integrated), z = colour phase
// (color_speed integrated), w = this frame's delta in seconds.
vec4 clockPass(void) {
    vec4 prev = texelFetch(clock, ivec2(0), 0);
    // A cleared target (first frame, or a rebuilt context) restarts at the
    // scene time.
    if (prev.x <= 0.0) { return vec4(TIME, TIME * speed, TIME * color_speed * 0.02, 1.0 / 60.0); }
    float dt = stepDt(prev.x);
    return vec4(TIME, prev.y + dt * speed, prev.z + dt * color_speed * 0.02, dt);
}

// Flow in screen-height units. Long drags: lanes of shear along a slowly
// turning direction, so paint is pulled out into long combed strokes. On
// top, the curl of a noise potential swirls it (one octave: the drags give
// the fine direction, and a second octave cost 15% of the scene). Both are
// divergence-free, so paint stretches and folds instead of piling up.
vec2 flowAt(vec2 p, float t) {
    vec2 q = p * flow_scale + vec2(t * 0.031, -t * 0.047);
    const float e = 0.06;
    float s0 = _noise(q);
    float sx = _noise(q + vec2(e, 0.0));
    float sy = _noise(q + vec2(0.0, e));
    vec2 curl = vec2(sy - s0, s0 - sx) / e;
    float a = 0.25 + 0.9 * sin(t * 0.037) + 0.35 * sin(t * 0.093 + 1.0);
    vec2 dir = vec2(cos(a), sin(a));
    float across = dot(p, vec2(-dir.y, dir.x));
    float lane = sin(across * 2.3 + t * 0.05) + 0.6 * sin(across * 5.3 - t * 0.07 + 2.0);
    return curl * (1.0 - 0.35 * min(drag, 1.5)) + dir * (0.30 + 0.75 * drag * lane);
}

// Tint target: broad islands, contrast-stretched so they span the range.
float tintAt(vec2 p, float t) {
    vec2 q = p * 1.35 + vec2(t * 0.02, 3.1 - t * 0.015);
    float n = _fbm(q + 1.2 * vec2(_noise(q * 1.7 + t * 0.05), _noise(q * 1.7 - 4.0)), 3.0);
    return clamp((n - 0.5) * 3.2 + 0.5, 0.0, 1.0);
}

vec4 flowPass(void) {
    float t = texelFetch(clock, ivec2(0), 0).y;
    vec2 p = _uvc;
    return vec4(flowAt(p, t), tintAt(p, t), _noise(p * 2.6 + vec2(t * 0.03, 1.7)));
}

vec4 fieldPass(void) {
    float asp = RENDERSIZE.x / RENDERSIZE.y;
    vec2 px = 1.0 / RENDERSIZE;
    vec2 p = _uvc;
    vec4 clk = texelFetch(clock, ivec2(0), 0);
    float t = clk.y;
    // Motion per frame scales with the frame delta, so the drag covers the
    // same distance per second at any frame rate.
    float frames = clamp(clk.w * 60.0, 0.25, 4.0);
    float bass = clamp(syn_BassLevel, 0.0, 1.0) * reactivity;
    float hit = clamp(syn_BassHits, 0.0, 1.0) * reactivity;
    float high = clamp(syn_HighLevel, 0.0, 1.0) * reactivity;

    // Broad islands decide colour, texture roughness and smear length.
    vec4 ff = texture(flowfield, _uv);
    float tint = ff.b;
    float rough = smoothstep(0.3, 0.7, tint);

    // Advect: sample upstream along the flow.
    vec2 vel = ff.rg * flow * 0.0032 * frames * (1.0 + 1.6 * bass + 0.9 * hit);
    vec2 uvS = _uv - vel / vec2(asp, 1.0);
    // Direction fades to nothing where the flow stalls, so no hatching there.
    vec2 vT = vel * RENDERSIZE;
    vec2 dirT = vT / max(length(vT), 0.7);
    vec2 stepT = dirT * px;
    vec2 perpT = vec2(-dirT.y, dirT.x) * px;
    float len = smear * (0.45 + 0.9 * rough) * (1.0 + 1.5 * bass);

    // The centre is two taps half a texel either side across the flow, a
    // [1 2 1] blur across it that removes the pixel-period (Nyquist) mode.
    // Without it, where the flow is axis-aligned every tap lands on a texel
    // centre, nothing filters across the flow, and the unsharp below grew
    // one-pixel rows (horizontal flow) or columns (vertical flow).
    vec4 a = 0.5 * (texture(field, uvS - 0.5 * perpT) + texture(field, uvS + 0.5 * perpT));
    if (a.a < 0.5) {
        // First frame, or the target was resized: seed with developed noise.
        float h0 = _fbm(p * 9.0 + 5.0, 3.0);
        return vec4(clamp((h0 - 0.5) * 2.5 + 0.5, 0.0, 1.0), tint, 0.0, 1.0);
    }
    vec4 b = texture(field, uvS - stepT * len);
    vec4 c = texture(field, uvS + stepT * len);
    vec4 l = texture(field, uvS - perpT);
    vec4 r = texture(field, uvS + perpT);

    float h = 0.5 * a.r + 0.25 * (b.r + c.r);
    float g = 0.5 * a.g + 0.25 * (b.g + c.g);
    // Unsharp across the flow keeps the fibres crisp against the smear. With
    // the blurred centre the pair passes every frequency at a gain below 1
    // (0.9 at the pixel period, 0.95 at two pixels, toward 1 for broad
    // shapes), so it sharpens what the advection blurs and never grows a mode.
    h += SHARPEN * clamp(a.r - 0.5 * (l.r + r.r), -0.12, 0.12);

    // Fresh striations: fine isotropic noise. The smear along the flow
    // (below, next frame) stretches it into fibres that follow the
    // streamlines, so no fibre frame has to be derived from the flow.
    float fibHi = fibers * (1.0 + 0.6 * high);
    float fib = _noise(p * fibHi + vec2(t * 0.15, 3.0));
    h += (fib - 0.5) * (0.03 + 0.10 * rough + 0.05 * high);
    g += (fib - 0.5) * 0.03;
    // Large islands of height, so the fine fibres ride on broad shapes.
    h = mix(h, ff.a * 1.3 - 0.15, 0.03);

    // Bubbles: one disc per grid cell, born once per cycle where a slow
    // cluster mask is high, then dragged away by the flow like everything
    // else. Birth is on the clock, once per period at any frame rate.
    // Most cells are not born this frame, so the rest of the work is skipped
    // for them.
    const float cells = 20.0;
    vec2 bp = p * cells + vec2(0.0, 7.0);
    vec2 cell = floor(bp);
    vec2 f = fract(bp) - 0.5;
    vec4 rn = _hash42(cell);
    const float period = 0.4;
    float ph = TIME + rn.z * period;
    float born = step(mod(ph, period), clk.w);
    if (born > 0.0) {
        float cyc = floor(ph / period);
        float roll = _hash12(cell + cyc * 17.3);
        float cluster = smoothstep(0.52, 0.66, _noise(p * 3.1 + vec2(-t * 0.04, t * 0.03)));
        float want = step(roll, bubbles * (0.04 + 0.96 * cluster));
        float rad = 0.30 + 0.16 * rn.w;
        float dc = length(f - (rn.xy - 0.5) * 0.3);
        float disc = want * (1.0 - smoothstep(rad - 0.05, rad, dc));
        float core = smoothstep(rad * 0.35, rad * 0.75, dc);
        h = mix(h, fract(roll * 7.0 + cyc * 0.37 + 0.3 * core), disc);
        g = mix(g, fract(roll * 5.0 + 0.3), disc * 0.6);
    }

    // Tint follows slowly moving islands.
    g = mix(g, tint, (0.012 + 0.01 * hit) * frames);

    // Folding instead of clamping keeps fine detail alive at the extremes.
    return vec4(_mirror(h), clamp(g, 0.0, 1.0), 0.0, 1.0);
}

// The acid palette. Height steps inside an island jump three slots, so
// every island mixes acid green and lemon with blue, violet, magenta and
// cyan accents, as the reference does; greens hold four slots of ten.
vec3 acid(float x) {
    const vec3 P[10] = vec3[10](
        vec3(0.42, 1.00, 0.08), vec3(1.00, 0.20, 0.68), vec3(1.00, 0.93, 0.12),
        vec3(0.24, 0.30, 1.00), vec3(0.62, 1.00, 0.16), vec3(0.08, 0.90, 0.92),
        vec3(0.86, 1.00, 0.10), vec3(0.60, 0.28, 1.00), vec3(0.18, 1.00, 0.36),
        vec3(0.62, 0.84, 1.00));
    return P[int(floor(fract(x) * 10.0))];
}

vec4 postPass(void) {
    vec2 uv = _uv;
    float high = clamp(syn_HighLevel, 0.0, 1.0) * reactivity;

    // The field is the output's size, so its texels sit exactly on the
    // pixels and a point fetch (wrapped at the edge, as the sampler was) is
    // what a bilinear tap at a texel centre returns.
    ivec2 ip = ivec2(_xy);
    ivec2 sz = ivec2(RENDERSIZE);
    ivec2 lo = ivec2(ip.x > 0 ? ip.x - 1 : sz.x - 1, ip.y > 0 ? ip.y - 1 : sz.y - 1);
    ivec2 hi = ivec2(ip.x < sz.x - 1 ? ip.x + 1 : 0, ip.y < sz.y - 1 ? ip.y + 1 : 0);
    vec2 c = texelFetch(field, ip, 0).rg;
    float hl = texelFetch(field, ivec2(lo.x, ip.y), 0).r;
    float hr = texelFetch(field, ivec2(hi.x, ip.y), 0).r;
    float hd = texelFetch(field, ivec2(ip.x, lo.y), 0).r;
    float hu = texelFetch(field, ivec2(ip.x, hi.y), 0).r;
    vec2 grad = vec2(hr - hl, hu - hd);
    float edge = length(grad);

    // Posterise: tint picks the island, height steps within it.
    float v = c.g * ISLANDS + c.r * bands;
    float idx = floor(v);
    float fr = fract(v);
    float phase = hue + texelFetch(clock, ivec2(0), 0).z + syn_BassTime * 0.19 * reactivity;
    // Islands take far-apart colours; the height steps inside one jump three
    // palette slots, so accents sit hard against the island's own colour.
    float hh = fract(floor(c.g * ISLANDS) * 0.381966 + floor(c.r * bands) * 0.3 + phase);
    float val = 0.84 + 0.16 * fract(idx * 0.3141 + 0.2);
    vec3 col = acid(hh);
    col = mix(col, vec3(1.0), 0.22 * step(0.8, fract(idx * 0.7071)));
    col *= val * (0.8 + 0.3 * fr);

    // Relief: lit from the upper left, bright pastel on the lit slope.
    float lit = dot(grad, vec2(-0.6, 0.8)) * relief * 9.0;
    col *= 1.0 + 0.8 * clamp(lit, -0.8, 0.8);
    col = mix(col, vec3(1.0), clamp(lit - 0.4, 0.0, 1.0) * 0.4);

    // Contours: where a colour step lands, and where the field changes fast.
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
    if (PASSINDEX == 0) { return clockPass(); }
    if (PASSINDEX == 1) { return flowPass(); }
    if (PASSINDEX == 2) { return fieldPass(); }
    return postPass();
}
