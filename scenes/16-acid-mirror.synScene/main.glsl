// Acid Mirror: original procedural interpretation of the owner's mirrored
// liquid-paint video (https://www.youtube.com/watch?v=DE5lQ1FN8zY).
//
// Three passes, as TouchDesigner does it: a DOUBLE_BUFFER feedback pass
// advects paint along a divergence-free flow and injects fresh paint; a
// one-texel clock pass supplies a real frame delta; the final pass mirrors the
// left half of the feedback buffer and lights it as glossy enamel.
//
// The buffer stores only scalars, so no grey mush when paint smears:
//   r = colour index along the acid palette (0..1, never wraps),
//   g = ink density (sharpened every frame, so edges stay crisp and the
//       black voids stay black).

// ---------------------------------------------------------------- shared

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
    vec3 c = vec3(0.05, 0.92, 0.10);
    c = mix(c, vec3(0.00, 1.00, 0.45), smoothstep(0.00, 0.12, x));
    c = mix(c, vec3(0.62, 1.00, 0.02), smoothstep(0.12, 0.26, x));
    c = mix(c, vec3(1.00, 0.88, 0.00), smoothstep(0.26, 0.38, x));
    c = mix(c, vec3(1.00, 0.10, 0.70), smoothstep(0.38, 0.50, x));
    c = mix(c, vec3(0.50, 0.08, 1.00), smoothstep(0.50, 0.58, x));
    c = mix(c, vec3(0.02, 0.30, 1.00), smoothstep(0.58, 0.66, x));
    c = mix(c, vec3(0.00, 0.92, 1.00), smoothstep(0.66, 0.74, x));
    c = mix(c, vec3(0.05, 0.92, 0.10), smoothstep(0.74, 0.88, x));
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

// What the world wants this frame at p. Ink (g) is added where a slow noise is
// high and carved where it is low, so paint flowing into a void is cut off
// and the black stays black. Dye sources are fixed in space while the flow
// moves through them, so they trail off downstream as long streaks.
void paintSource(vec2 p, float t, out float s, out float add, out float cut,
                 out float acc, out float vein) {
    float kick = clamp(syn_BassHits, 0.0, 1.0) * reactivity;
    vec2 q = p * scale;
    float n = _fbm(vec3(q * 1.8, t * 0.11), 3.0);
    float thr = 0.44 - (ink - 0.5) * 0.16;
    add = smoothstep(thr, thr + 0.06, n);
    cut = 1.0 - smoothstep(thr - 0.07, thr, n);
    add *= 1.0 - cut;

    // Banded colour: contours of a warped noise, mostly greens and lemon,
    // with accent colours where a second noise peaks.
    float c = _fbm(q * 3.4 + vec2(n * 2.0, -t * 0.06), 3.0);
    float accent = _fbm(q * 2.6 + vec2(7.0, -t * 0.08), 2.0);
    s = 0.22 * pingPong(c * 7.0 + 0.5 * n);
    s = mix(s, 0.34 + 0.6 * pingPong(c * 4.0), smoothstep(0.53, 0.62, accent));
    s += 0.12 * kick;

    // Thin dye lines (accent colour) and ink-black veins.
    float h1 = _fbm(q * 4.5 + vec2(t * 0.03, 11.0), 2.0);
    acc = (1.0 - smoothstep(0.0, 0.03, abs(h1 - 0.5))) * detail;
    // Fine grain that the flow combs into directional streaks.
    vein = _noise(q * 15.0 + vec2(t * 0.07, 23.0));

    // Two pens drawing near the axis; the mirror makes them eyes.
    float pr = 0.055 * (1.0 + 0.6 * kick);
    vec2 pen = vec2(-0.20 - 0.10 * sin(t * 0.23), 0.16 * sin(t * 0.31 + 1.0));
    float dp = length(p - pen);
    float pm = smoothstep(pr, pr * 0.55, dp);
    float ps = mix(0.84, 0.28, smoothstep(pr * 0.25, pr * 0.6, dp));
    s = mix(s, ps, pm);
    vec2 pen2 = vec2(-0.40 + 0.12 * sin(t * 0.17 + 2.0), -0.22 + 0.13 * sin(t * 0.27));
    float dq = length(p - pen2);
    float qm = smoothstep(pr * 1.4, pr * 0.7, dq);
    float qs = mix(0.58, 0.18, smoothstep(pr * 0.3, pr * 1.0, dq));
    s = mix(s, qs, qm);
    float pens = max(pm, qm);
    add = max(add, pens);
    cut *= 1.0 - pens;
    acc *= 1.0 - pens;
}

vec4 paintPass() {
    vec4 clk = texelFetch(clock, ivec2(0), 0);
    float dt = stepDt(clk.x);
    float rate = flowRate();
    float T = clk.y + dt * rate;
    float A = 2.0 * RENDERSIZE.x / RENDERSIZE.y;
    vec2 pc = (vec2(_uv.x * 0.5, _uv.y) - 0.5) * vec2(A, 1.0);

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
    paintSource(pc, T, ss, add, cut, acs, vns);
    float first = (FRAMECOUNT < 2.0) ? 1.0 : 0.0;
    d = max(d, add);
    d *= exp(-dt * 16.0 * cut) * (1.0 - first * cut);
    s = mix(s, ss, max(add * (1.0 - exp(-dt * 0.9)), first));
    acc = mix(acc, acs, 1.0 - exp(-dt * 3.5));
    vein = mix(vein, vns, 1.0 - exp(-dt * 1.3));

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

// .x ink thickness, .y colour index through the band fold.
vec2 surfaceAt(vec2 uv) {
    vec4 c = texture(paintBuf, foldUV(uv));
    return vec2(smoothstep(0.1, 0.9, c.g) + 1.2 * detail * c.a, pingPong(c.r * bands + 2.2 * detail * (c.a - 0.5)));
}

vec4 finalPass() {
    vec4 c = texture(paintBuf, foldUV(_uv));
    float s = c.r;
    float d = c.g;
    float a = smoothstep(0.46, 0.54, d);

    vec2 e = 1.25 / RENDERSIZE;
    vec2 s0 = surfaceAt(_uv);
    vec2 sE = surfaceAt(_uv + vec2(e.x, 0.0));
    vec2 sW = surfaceAt(_uv - vec2(e.x, 0.0));
    vec2 sN = surfaceAt(_uv + vec2(0.0, e.y));
    vec2 sS = surfaceAt(_uv - vec2(0.0, e.y));
    vec2 grad = vec2(sE.x - sW.x + 0.55 * (sE.y - sW.y), sN.x - sS.x + 0.55 * (sN.y - sS.y));
    vec3 n = normalize(vec3(-grad * relief * 4.5, 1.0));
    vec3 light = normalize(vec3(-0.45, 0.55, 0.7));
    float diff = max(dot(n, light), 0.0);
    vec3 refl = reflect(-light, n);
    float spec = pow(max(refl.z, 0.0), 48.0);
    float sheen = pow(max(refl.z, 0.0), 6.0);
    float rim = pow(1.0 - n.z, 2.0);

    float idx = pingPong(s * bands + hue - 0.06 + 1.7 * detail * (c.a - 0.5) + 0.15 * n.x * relief + 0.45 * smoothstep(0.3, 0.75, c.b));
    vec3 col = acidPalette(idx);
    col = col * col * (3.0 - 2.0 * col);
    col *= 0.16 + 1.05 * diff + 0.25 * sheen;
    // Ink lines in the grooves, and a dark rim at the edge of the paint.
    float lap = sE.y + sW.y + sN.y + sS.y - 4.0 * s0.y + 0.5 * (sE.x + sW.x + sN.x + sS.x - 4.0 * s0.x);
    col *= 1.0 - 0.85 * smoothstep(0.02, 0.08, lap);
    col *= 0.5 + 0.5 * smoothstep(0.45, 0.95, d);
    col += vec3(0.8, 1.0, 0.95) * (spec * 0.9 + rim * 0.2);
    col *= a * brightness * (1.0 + 0.12 * clamp(syn_BassLevel, 0.0, 1.0) * reactivity);
    return vec4(clamp(col, 0.0, 1.0), 1.0);
}

vec4 renderMain(void) {
    if (PASSINDEX == 0) { return paintPass(); }
    if (PASSINDEX == 1) { return clockPass(); }
    return finalPass();
}
