// Acid Mirror: original procedural interpretation of the owner's mirrored
// liquid-paint video (https://www.youtube.com/watch?v=DE5lQ1FN8zY).
//
// Four passes, as TouchDesigner does it: a DOUBLE_BUFFER feedback pass
// advects paint along a divergence-free flow and injects fresh paint; a
// one-texel clock pass supplies a real frame delta; the final pass mirrors the
// left half of the feedback buffer and lights it as glossy enamel. The paint
// is injected from noise fields; the three broad ones (the ink mask, the
// accent bands and the dye lines, about 70% of the noise arithmetic) are
// evaluated first, in a pass at half the feedback buffer's size, and the
// feedback pass reads them back with bilinear filtering and applies the sharp
// thresholds itself. The finer colour-band and grain noise stay at full size.
//
// The buffer stores only scalars, so no grey mush when paint smears:
//   r = colour index along the acid palette (0..1, never wraps),
//   g = ink density (sharpened every frame, so edges stay crisp and the
//       black voids stay black).

// ---------------------------------------------------------------- shared

// Iso-lines of the combed grain per unit of it, for the hairline streaks.
const float STREAKS = 44.0;

// Seconds that advance the flow: speed control, mids make it swirl faster.
float flowRate() {
    return speed * (1.0 + 0.9 * clamp(syn_MidLevel, 0.0, 1.0) * reactivity);
}

float stepDt(float prevTime) {
    float d = TIME - prevTime;
    return (d > 0.0 && d < 0.2) ? d : 1.0 / 60.0;
}

// Ping-pong fold: s sweeps back and forth, so the palette never jumps.
float pingPong(float x) { return 1.0 - abs(1.0 - 2.0 * fract(x * 0.5)); }

// Acid path: greens and lemon dominate, then amber, magenta, violet,
// cobalt and cyan streaks, and back to green.
vec3 acidPalette(float x) {
    // The eight blends cover disjoint ranges of x, so each earlier one is
    // already 1 and each later one still 0 wherever a blend is in progress:
    // the chain of mixes is the first colour plus each blend's weight times
    // its step, which is the same arithmetic with one multiply-add per stop.
    vec3 c = vec3(0.05, 0.92, 0.10);
    c += smoothstep(0.00, 0.12, x) * vec3(-0.05, 0.08, 0.35);
    c += smoothstep(0.12, 0.26, x) * vec3(0.62, 0.00, -0.43);
    c += smoothstep(0.26, 0.38, x) * vec3(0.38, -0.12, -0.02);
    c += smoothstep(0.38, 0.50, x) * vec3(0.00, -0.78, 0.70);
    c += smoothstep(0.50, 0.58, x) * vec3(-0.50, -0.02, 0.30);
    c += smoothstep(0.58, 0.66, x) * vec3(-0.48, 0.22, 0.00);
    c += smoothstep(0.66, 0.74, x) * vec3(-0.02, 0.62, 0.00);
    c += smoothstep(0.74, 0.88, x) * vec3(0.05, 0.00, -0.90);
    return c;
}

// ---------------------------------------------------------------- flow

// Velocity from a stream function psi = sum a sin(k.p + w t): the curl of
// it conserves area, so paint stretches and folds instead of piling up.
vec2 waveV(vec2 p, vec2 k, float ph, float amp) {
    return amp * cos(dot(k, p) + ph) * vec2(k.y, -k.x) / length(k);
}

vec2 flowVel(vec2 p, float t) {
    vec2 q = p * scale;
    q += 0.30 * vec2(sin(q.y * 2.1 + t * 0.21), cos(q.x * 1.7 - t * 0.17));
    vec2 v = waveV(q, vec2(1.9, 0.7), t * 0.37, 1.00);
    v += waveV(q, vec2(-0.8, 2.6), t * 0.29 + 1.7, 0.80);
    v += waveV(q, vec2(3.9, -2.2), -t * 0.52 + 4.0, 0.55);
    v += waveV(q, vec2(-5.1, 4.4), t * 0.61 + 2.0, 0.35);
    v += waveV(q, vec2(7.5, 3.1), -t * 0.83 + 0.5, 0.20);
    return v;
}

// ---------------------------------------------------------------- paint

// The broad noise fields behind the paint, at p: .x the ink mask noise, .y the
// accent noise and .z the dye-line noise. Evaluated at a quarter of the
// feedback buffer's pixels.
vec4 sourceFields(vec2 p, float t) {
    vec2 q = p * scale;
    float n = _fbm(vec3(q * 1.8, t * 0.11), 3.0);
    float accent = _fbm(q * 2.6 + vec2(7.0, -t * 0.08), 2.0);
    float h1 = _fbm(q * 4.5 + vec2(t * 0.03, 11.0), 2.0);
    return vec4(n, accent, h1, 0.0);
}

// What the world wants this frame at p. Ink (g) is added where a slow noise is
// high and carved where it is low, so paint flowing into a void is cut off
// and the black stays black. Dye sources are fixed in space while the flow
// moves through them, so they trail off downstream as long streaks.
// f holds the noise fields from sourceFields.
void paintSource(vec2 p, float t, vec4 f, out float s, out float add, out float cut,
                 out float acc, out float vein) {
    float kick = clamp(syn_BassHits, 0.0, 1.0) * reactivity;
    vec2 q = p * scale;
    float n = f.x;
    float thr = 0.44 - (ink - 0.5) * 0.16;
    add = smoothstep(thr, thr + 0.06, n);
    cut = 1.0 - smoothstep(thr - 0.07, thr, n);
    add *= 1.0 - cut;

    // Banded colour: contours of a warped noise, mostly greens and lemon,
    // with accent colours where a second noise peaks.
    float c = _fbm(q * 3.4 + vec2(n * 2.0, -t * 0.06), 3.0);
    float accent = f.y;
    s = 0.22 * pingPong(c * 7.0 + 0.5 * n);
    s = mix(s, 0.34 + 0.6 * pingPong(c * 4.0), smoothstep(0.53, 0.62, accent));
    s += 0.12 * kick;

    // Thin dye lines (accent colour) and ink-black veins.
    acc = (1.0 - smoothstep(0.0, 0.03, abs(f.z - 0.5))) * detail;
    // Fine grain that the flow combs into directional streaks.
    vein = _noise(q * 15.0 + vec2(t * 0.07, 23.0));

    // Two pens drawing near the axis; the mirror makes them eyes.
    float pr = 0.055 * (1.0 + 0.6 * kick);
    vec2 pen = vec2(-0.20 - 0.10 * sin(t * 0.23), 0.16 * sin(t * 0.31 + 1.0));
    float dp = length(p - pen);
    float pm = 1.0 - smoothstep(pr * 0.55, pr, dp);
    float ps = mix(0.84, 0.28, smoothstep(pr * 0.25, pr * 0.6, dp));
    s = mix(s, ps, pm);
    vec2 pen2 = vec2(-0.40 + 0.12 * sin(t * 0.17 + 2.0), -0.22 + 0.13 * sin(t * 0.27));
    float dq = length(p - pen2);
    float qm = 1.0 - smoothstep(pr * 0.7, pr * 1.4, dq);
    float qs = mix(0.58, 0.18, smoothstep(pr * 0.3, pr * 1.0, dq));
    s = mix(s, qs, qm);
    float pens = max(pm, qm);
    add = max(add, pens);
    cut *= 1.0 - pens;
    acc *= 1.0 - pens;
}

// The flow time and frame delta the feedback pass and the source fields both
// use: both run before the clock pass, so they read last frame's clock. A
// cleared clock (first frame, or a resize) restarts at the scene time, as the
// clock pass will this frame.
float flowTime(out float dt) {
    vec4 clk = texelFetch(clock, ivec2(0), 0);
    dt = stepDt(clk.x);
    return (clk.x > 0.0) ? clk.y + dt * flowRate() : TIME * flowRate();
}

// A pixel's place in screen heights, centred on the axis, for a pass whose
// target holds the left half of the picture.
vec2 leftHalfPos() {
    float A = 2.0 * RENDERSIZE.x / RENDERSIZE.y;
    return (vec2(_uv.x * 0.5, _uv.y) - 0.5) * vec2(A, 1.0);
}

vec4 sourcesPass() {
    float dt;
    float T = flowTime(dt);
    return sourceFields(leftHalfPos(), T);
}

vec4 paintPass() {
    float dt;
    float T = flowTime(dt);
    float rate = flowRate();
    float A = 2.0 * RENDERSIZE.x / RENDERSIZE.y;
    vec2 pc = leftHalfPos();

    // Semi-Lagrangian advection with a midpoint step.
    float k = dt * rate * 0.22 * swirl;
    vec2 mid = pc - 0.5 * k * flowVel(pc, T);
    vec2 src = pc - k * flowVel(mid, T);
    vec2 bu = vec2((src.x / A + 0.5) * 2.0, src.y + 0.5);
    bu.x = 1.0 - abs(1.0 - bu.x);      // reflect at the mirror axis
    vec2 px = 1.0 / RENDERSIZE;
    vec4 prev = texture(paintBuf, bu);
    float s = prev.r;
    float d = prev.g;
    float acc = prev.b;
    float vein = prev.a;

    // Shock-filter sharpening of the colour index: bilinear advection blurs
    // it a little every frame, this steepens it back without overshoot.
    float sa = texture(paintBuf, bu + vec2(px.x, 0.0)).r;
    float sb = texture(paintBuf, bu - vec2(px.x, 0.0)).r;
    float sc = texture(paintBuf, bu + vec2(0.0, px.y)).r;
    float sd = texture(paintBuf, bu - vec2(0.0, px.y)).r;
    float lo = min(min(sa, sb), min(min(sc, sd), s));
    float hi = max(max(sa, sb), max(max(sc, sd), s));
    s = clamp(s + 0.3 * (s - 0.25 * (sa + sb + sc + sd)), lo, hi);

    float frames = dt * 60.0;
    float ss, add, cut, acs, vns;
    paintSource(pc, T, texture(sourcesBuf, _uv), ss, add, cut, acs, vns);
    // A cleared buffer (the first frame, or a resize or render-scale change
    // reallocating it) reads all zeros, which written paint never is (the
    // grain is never exactly 0): take the sources whole, so the picture is
    // back in one frame instead of growing back over seconds.
    float first = (FRAMECOUNT < 2.0 || prev == vec4(0.0)) ? 1.0 : 0.0;
    d = max(d, add);
    d *= exp(-dt * 16.0 * cut) * (1.0 - first * cut);
    s = mix(s, ss, max(add * (1.0 - exp(-dt * 0.9)), first));
    acc = mix(acc, acs, max(1.0 - exp(-dt * 3.5), first));
    vein = mix(vein, vns, max(1.0 - exp(-dt * 1.3), first));

    // Keep edges crisp (threshold dynamics): faint or thin paint resolves to
    // either solid or nothing.
    d = clamp((d - 0.5) * pow(1.5, frames) + 0.5, 0.0, 1.0);
    return vec4(clamp(s, 0.0, 1.0), d, clamp(acc, 0.0, 1.0), clamp(vein, 0.0, 1.0));
}

vec4 clockPass() {
    vec4 prev = texelFetch(clock, ivec2(0), 0);
    // A cleared target (first frame, or a resize) restarts at the scene time.
    if (prev.x <= 0.0) { return vec4(TIME, TIME * flowRate(), 0.0, 1.0); }
    float dt = stepDt(prev.x);
    return vec4(TIME, prev.y + dt * flowRate(), 0.0, 1.0);
}

// ---------------------------------------------------------------- output

// The buffer holds the left half of the picture; fold the right onto it.
vec2 foldUV(vec2 uv) {
    uv.y = mix(uv.y, 0.5 - abs(uv.y - 0.5), mirror_y);
    return vec2(1.0 - abs(1.0 - 2.0 * uv.x), uv.y);
}

// .x ink thickness, .y colour index through the band fold, .z the streak
// coordinate (colour index scaled, plus a little grain).
vec3 surface(vec4 c) {
    return vec3(0.45 * smoothstep(0.2, 0.8, c.g) + 1.2 * detail * c.a,
                pingPong(c.r * bands + 2.2 * detail * (c.a - 0.5)),
                c.r * STREAKS + 1.2 * c.a);
}

vec3 surfaceAt(vec2 uv) { return surface(texture(paintBuf, foldUV(uv))); }

vec4 finalPass() {
    vec4 c = texture(paintBuf, foldUV(_uv));
    float s = c.r;
    float d = c.g;
    // Coverage over one pixel of the paint edge, so the cut to black is crisp.
    float a = clamp((d - 0.5) / max(fwidth(d), 0.01) + 0.5, 0.0, 1.0);

    vec2 e = 1.25 / RENDERSIZE;
    vec3 s0 = surface(c);
    vec3 sE = surfaceAt(_uv + vec2(e.x, 0.0));
    vec3 sW = surfaceAt(_uv - vec2(e.x, 0.0));
    vec3 sN = surfaceAt(_uv + vec2(0.0, e.y));
    vec3 sS = surfaceAt(_uv - vec2(0.0, e.y));
    vec2 grad = vec2(sE.x - sW.x + 0.55 * (sE.y - sW.y), sN.x - sS.x + 0.55 * (sN.y - sS.y));
    vec3 n = normalize(vec3(-grad * relief * 4.5, 1.0));
    vec3 light = normalize(vec3(-0.45, 0.55, 0.7));
    float diff = max(dot(n, light), 0.0);
    vec3 refl = reflect(-light, n);
    // Fixed integer powers by repeated squaring, not exp2(log2).
    float rz = max(refl.z, 0.0);
    float rz2 = rz * rz;
    float rz4 = rz2 * rz2;
    float rz8 = rz4 * rz4;
    float rz16 = rz8 * rz8;
    float spec = rz16 * rz16 * rz16;
    float sheen = rz4 * rz2;
    float rim = (1.0 - n.z) * (1.0 - n.z);

    float idx = pingPong(s * bands + hue - 0.06 + 1.7 * detail * (c.a - 0.5) + 0.15 * n.x * relief + 0.45 * smoothstep(0.3, 0.75, c.b));
    vec3 col = acidPalette(idx);
    col = col * col * (3.0 - 2.0 * col);
    col *= 0.16 + 1.05 * diff + 0.25 * sheen;
    // Ink lines in the grooves, and a dark rim at the edge of the paint.
    float lap = sE.y + sW.y + sN.y + sS.y - 4.0 * s0.y + 0.5 * (sE.x + sW.x + sN.x + sS.x - 4.0 * s0.x);
    col *= 1.0 - 0.85 * smoothstep(0.02, 0.08, lap);

    // Hairline streaks: iso-lines of the colour index. The flow folds and
    // stretches that index along itself, so its iso-lines run with the paint
    // in nested bundles, like dragged pigment. Each line is about two pixels
    // wide whatever the spacing (distance to the line over its rate per
    // pixel, measured across 2.5 pixels so it is smooth), takes the accent
    // colours in turn, and fades where lines would crowd into moire, which
    // also leaves the hard colour steps alone.
    float hv = s0.z;
    float hw = max(length(vec2(sE.z - sW.z, sN.z - sS.z)) * 0.4, 1e-4);
    float dl = abs(fract(hv) - 0.5) / hw;
    float line = (1.0 - smoothstep(0.6, 1.4, dl)) * (1.0 - smoothstep(0.18, 0.4, hw));
    if (line > 0.0) {
        float k = floor(hv);
        vec3 lineCol = acidPalette(0.40 + 0.34 * fract(k * 0.618034));
        lineCol = mix(lineCol, vec3(1.0), step(0.78, fract(k * 0.371)));
        col = mix(col, lineCol * (0.55 + 0.6 * diff), line * streaks * smoothstep(0.55, 0.8, d));
    }

    // A thin dark rim at the edge of the paint, then a crisp cut to black.
    col *= 0.3 + 0.7 * smoothstep(0.5, 0.6, d);
    col += vec3(0.8, 1.0, 0.95) * (spec * 0.9 + rim * 0.2);
    col *= a * brightness * (1.0 + 0.12 * clamp(syn_BassLevel, 0.0, 1.0) * reactivity);
    return vec4(clamp(col, 0.0, 1.0), 1.0);
}

vec4 renderMain(void) {
    if (PASSINDEX == 0) { return sourcesPass(); }
    if (PASSINDEX == 1) { return paintPass(); }
    if (PASSINDEX == 2) { return clockPass(); }
    return finalPass();
}
