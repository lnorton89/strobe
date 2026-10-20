// Blacklight Carpet: original procedural interpretation of the confetti
// carpet in 1990s arcades and bowling alleys, lit by UV tubes. Fluorescent
// squares, rings, triangles, diamonds, sprinkles and coiled streamers glow on
// black pile.
//
// Two passes: a one-texel clock integrates the flow time and the drift
// offset, so the drift path can turn and mids can surge without the pattern
// jumping. Then the carpet is drawn analytically in five layers (pile,
// sprinkles, small confetti, streamers, large confetti) that drift at
// different rates for parallax. Every piece is a signed distance with a
// smoothstep over the pixel footprint, and the pile is never finer than a few
// pixels, so nothing shimmers as it moves.

const float TAU = 6.2831853;

float flowRate() {
    return speed * (1.0 + 0.8 * clamp(syn_MidLevel, 0.0, 1.0) * reactivity);
}

float stepDt(float prevTime) {
    float d = TIME - prevTime;
    return (d > 0.0 && d < 0.2) ? d : 1.0 / 60.0;
}

// The straight part of the drift, integrated by the clock so `drift` and
// `speed` can change without the carpet jumping.
const vec2 DRIFT_DIR = vec2(-0.62, -0.78);

// x: scene time last frame, y: flow time, zw: straight drift offset.
vec4 clockPass() {
    vec4 prev = texelFetch(clock, ivec2(0), 0);
    float rate = flowRate();
    float vel = 0.035 * drift;
    // A cleared target (first frame, or a resize) restarts at the scene time.
    if (prev.x <= 0.0) {
        float t0 = TIME * rate;
        return vec4(TIME, t0, DRIFT_DIR * vel * t0);
    }
    float dt = stepDt(prev.x);
    return vec4(TIME, prev.y + dt * rate, prev.zw + DRIFT_DIR * (vel * rate * dt));
}

// The whole drift: the straight part plus a slow meander of flow time, so the
// path keeps turning (up to about 40 degrees either way, slowing and
// speeding) and still lands in the same place after a restart.
vec2 driftOffset(vec2 straight, float T) {
    return straight + drift * 0.6 * vec2(sin(T * 0.047), sin(T * 0.031 + 1.0));
}

// Fluorescent inks: the dyes that light up on black carpet under a UV tube.
vec3 inkAt(float k) {
    if (k < 0.5) { return vec3(1.00, 0.08, 0.52); }   // hot pink
    if (k < 1.5) { return vec3(1.00, 0.36, 0.03); }   // orange
    if (k < 2.5) { return vec3(1.00, 0.90, 0.06); }   // yellow
    if (k < 3.5) { return vec3(0.32, 1.00, 0.10); }   // green
    if (k < 4.5) { return vec3(0.05, 0.82, 1.00); }   // cyan
    if (k < 5.5) { return vec3(0.22, 0.30, 1.00); }   // blue
    return vec3(0.66, 0.18, 1.00);                    // violet
}

// One of seven inks; `hue` rotates them, crossfading between neighbours.
vec3 ink(float r) {
    float i = mod(floor(r * 7.0) + hue * 7.0, 7.0);
    float k = floor(i);
    return mix(inkAt(k), inkAt(mod(k + 1.0, 7.0)), smoothstep(0.0, 1.0, i - k));
}

mat2 rot(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, -s, s, c);
}

float sdBox(vec2 p, vec2 b) {
    vec2 d = abs(p) - b;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

// Isosceles triangle, tip at the origin, base at y = q.y (iq).
float sdTri(vec2 p, vec2 q) {
    p.x = abs(p.x);
    vec2 a = p - q * clamp(dot(p, q) / dot(q, q), 0.0, 1.0);
    vec2 b = p - q * vec2(clamp(p.x / q.x, 0.0, 1.0), 1.0);
    float s = -sign(q.y);
    vec2 d = min(vec2(dot(a, a), s * (p.x * q.y - p.y * q.x)),
                 vec2(dot(b, b), s * (p.y - q.y)));
    return -sqrt(d.x) * sign(d.y);
}

float sdRhombus(vec2 p, vec2 b) {
    p = abs(p);
    vec2 bb = b - 2.0 * p;
    float h = clamp((bb.x * b.x - bb.y * b.y) / dot(b, b), -1.0, 1.0);
    float d = length(p - 0.5 * b * vec2(1.0 - h, 1.0 + h));
    return d * sign(p.x * b.y + p.y * b.x - b.x * b.y);
}

// Distance to confetti piece `type` of size r, in its own rotated frame.
// The farthest point of any piece is within 1.4 r of its centre.
float confettiDist(vec2 p, float type, float r, float px) {
    float stroke = max(0.20 * r, 1.2 * px);
    if (type < 1.0) { return sdBox(p, vec2(r * 0.72)); }                          // square
    if (type < 2.0) { return abs(sdBox(p, vec2(r * 0.80))) - 0.5 * stroke; }      // open square
    if (type < 3.0) { return length(p) - r * 0.72; }                              // dot
    if (type < 4.0) { return abs(length(p) - r * 0.85) - 0.5 * stroke; }          // ring
    if (type < 5.0) {                                                              // linked rings
        float a = abs(length(p - vec2(r * 0.34, 0.0)) - r * 0.62) - 0.45 * stroke;
        float b = abs(length(p + vec2(r * 0.34, 0.0)) - r * 0.62) - 0.45 * stroke;
        return min(a, b);
    }
    if (type < 6.0) { return sdTri(p + vec2(0.0, 1.05 * r), vec2(0.92 * r, 1.58 * r)); }  // triangle
    if (type < 7.0) { return sdTri(p + vec2(0.0, 1.30 * r), vec2(0.40 * r, 1.95 * r)); }  // sliver
    return sdRhombus(p, vec2(r * 0.62, r * 1.0));                                         // diamond
}

// Under UV the dyed pile is not flat colour: tufts glow a little unevenly.
// The texture never gets finer than about three pixels.
float pileTexture(vec2 uv, float px) {
    return _noise(uv * min(420.0, 0.33 / px));
}

// One layer of confetti on a jittered grid. Each pixel looks at the four
// cells around it, so a piece can sit anywhere in its cell, wander and still
// glow across the border. Returns emission and coverage; the halo is added
// separately so it does not hide what is behind.
vec4 confettiLayer(vec2 uv, float cell, float seed, float density, float rMin, float rMax,
                   float T, float px, float pop, float gw, out vec3 halo) {
    halo = vec3(0.0);
    vec2 q = uv / cell;
    vec2 base = floor(q - 0.5);
    float pxc = px / cell;
    float gwc = gw / cell;
    vec3 emit = vec3(0.0);
    float cover = 0.0;
    for (int j = 0; j < 4; j++) {
        vec2 id = base + vec2(mod(float(j), 2.0), floor(float(j) * 0.5));
        vec4 h = _hash42(id + seed);
        if (h.x > density) { continue; }
        vec4 g = _hash42(id * 1.37 + seed + 19.1);
        float r = mix(rMin, rMax, h.w * h.w) * pop;
        float reach = 1.4 * r + 5.0 * gwc;
        vec2 p = q - (id + 0.5 + (g.xy - 0.5) * 0.5);
        // Out of reach wherever the wobble takes it: skip the trigonometry.
        if (dot(p, p) > (reach + 0.1) * (reach + 0.1)) { continue; }
        // Each piece drifts on a small loop around its spot.
        p -= 0.10 * vec2(sin(T * (0.21 + 0.20 * g.z) + g.w * TAU),
                         cos(T * (0.17 + 0.20 * g.w) + g.z * TAU));
        if (dot(p, p) > reach * reach) { continue; }
        p = rot(g.w * TAU + T * (g.z - 0.5) * 0.9 * spin) * p;
        float d = confettiDist(p, floor(h.y * 8.0), r, pxc);
        float a = 1.0 - smoothstep(-pxc, pxc, d);
        vec3 k = ink(h.z) * (0.80 + 0.25 * g.x);
        // A slightly brighter rim reads as fluorescence rather than paint.
        vec3 core = k * (0.82 + 0.30 * smoothstep(-0.25 * r, 0.0, d));
        emit = emit * (1.0 - a) + core * a;
        cover = max(cover, a);
        halo += k * exp(-max(d, 0.0) / gwc) * (1.0 - a);
    }
    // The fibre texture is common to every piece, so it is applied once,
    // and only where a piece covers the pixel.
    if (cover > 0.0) { emit *= 0.84 + 0.22 * pileTexture(uv, px); }
    return vec4(emit, cover);
}

// Tiny sprinkles: dots and dashes, densely packed, twinkling. They are small
// enough to stay inside their own cell, so each pixel looks at one cell. A
// sprinkle smaller than a pixel spreads over the smoothstep's footprint and
// dims instead of flickering.
vec3 sprinkleLayer(vec2 uv, float cell, float seed, float density, float T, float px, float high) {
    vec2 q = uv / cell;
    vec2 id = floor(q);
    vec4 h = _hash42(id + seed);
    if (h.x > density) { return vec3(0.0); }
    vec4 g = _hash42(id * 1.91 + seed + 5.3);
    vec2 p = q - (id + 0.5 + (g.xy - 0.5) * 0.5);
    float r = mix(0.07, 0.13, h.w);
    p = rot(g.z * TAU + T * (g.w - 0.5) * 0.6 * spin) * p;
    float pxc = px / cell;
    float d = h.y > 0.55 ? sdBox(p, vec2(r * 1.4, r * 0.42)) : length(p) - r * 0.75;
    float a = 1.0 - smoothstep(-pxc, pxc, d);
    float tw = 0.70 + 0.30 * sin(T * (0.9 + 2.2 * g.w) + g.z * TAU);
    tw += 1.6 * high * step(0.72, g.x) * (0.5 + 0.5 * sin(T * 7.0 + g.y * TAU));
    return ink(h.z) * a * tw;
}

// Coiled streamers: a curled ribbon is a helix, seen slightly from the
// side, so each turn draws a loop. Each stroke gets a true closest-point
// distance (below), for a clean edge and glow. The ribbon is a flat strip
// across the helix axis, so it is thin at each turnaround (seen edge on) and
// full on the strokes; the back strokes are dimmer. The coil rolls about its
// axis, so the loops travel along it, and the whole streamer bends and sways.
vec2 helixAt(float t, float pitch, float R, float ex) {
    return vec2(pitch * t / TAU + ex * sin(t), R * cos(t));
}

vec4 streamerLayer(vec2 uv, float cell, float seed, float density, float T, float px,
                   float pop, float gw, out vec3 halo) {
    halo = vec3(0.0);
    vec2 q = uv / cell;
    vec2 base = floor(q - 0.5);
    vec3 emit = vec3(0.0);
    float cover = 0.0;
    for (int j = 0; j < 4; j++) {
        vec2 id = base + vec2(mod(float(j), 2.0), floor(float(j) * 0.5));
        vec4 h = _hash42(id + seed);
        if (h.x > density) { continue; }
        vec4 g = _hash42(id * 1.53 + seed + 11.7);
        // World units from here on: the coil's size does not follow the cell.
        vec2 p = (q - (id + 0.5 + (g.xy - 0.5) * 0.4)) * cell;
        float L = mix(0.10, 0.15, h.w);
        float R = mix(0.016, 0.021, g.z) * pop;
        float hw = 0.0056 * pop;
        float reach = L + R + hw + 6.0 * gw;
        if (dot(p, p) > reach * reach) { continue; }
        p = rot(h.y * TAU + 0.35 * sin(T * 0.07 + g.z * TAU)) * p;
        float bend = 2.4 * (g.w - 0.5) + 1.0 * sin(T * 0.11 + g.x * TAU);
        p.y -= bend * p.x * p.x;
        if (abs(p.x) > L + R + hw + 6.0 * gw || abs(p.y) > R + hw + 6.0 * gw) { continue; }

        float pitch = mix(0.036, 0.046, g.y);
        float ex = 0.72 * R;
        // Rolling the coil about its axis moves the loops along it.
        float roll = T * (1.1 + 0.7 * g.y) * spin + h.x * TAU * 5.0;
        float tEnd = TAU * L / pitch;
        // Parameter t is measured from the streamer's middle; phase = t + roll.
        // Each half turn is one stroke, monotonic in y, so acos(y / R) seeds
        // it exactly on the curve's height and two Newton steps finish the
        // closest point. Three turns around the pixel cover every stroke
        // that can reach it; back strokes first, front strokes over them.
        float m0 = floor((TAU * p.x / pitch + roll) / TAU);
        float a0 = acos(clamp(p.y / R, -1.0, 1.0));
        vec3 kk = ink(h.z);
        for (int k = 0; k < 6; k++) {
            float front = k < 3 ? 0.0 : 1.0;
            float m = m0 + mod(float(k), 3.0) - 1.0;
            float lo = TAU * m + (front > 0.5 ? 0.0 : PI);
            float ph = front > 0.5 ? TAU * m + a0 : TAU * m + TAU - a0;
            float t0 = max(lo - roll, -tEnd);
            float t1 = min(lo + PI - roll, tEnd);
            if (t0 >= t1) { continue; }
            float t = clamp(ph - roll, t0, t1);
            for (int i = 0; i < 2; i++) {
                ph = t + roll;
                vec2 e = vec2(pitch * t / TAU + ex * sin(ph), R * cos(ph)) - p;
                vec2 d1 = vec2(pitch / TAU + ex * cos(ph), -R * sin(ph));
                vec2 d2 = vec2(-ex * sin(ph), -R * cos(ph));
                // Damped: far from the curve the second derivative can go
                // negative, and an undamped step would leave the stroke.
                float f2 = max(dot(d1, d1) + dot(e, d2), 0.5 * dot(d1, d1));
                t = clamp(t - clamp(dot(e, d1) / f2, -0.5, 0.5), t0, t1);
            }
            ph = t + roll;
            vec2 e = vec2(pitch * t / TAU + ex * sin(ph), R * cos(ph)) - p;
            vec2 tan1 = vec2(pitch / TAU + ex * cos(ph), -R * sin(ph));
            float d = length(e);
            // Strip across the axis: its apparent width follows the tangent's
            // vertical part, thin where the curve turns around.
            float facing = abs(tan1.y) / length(tan1);
            float taper = smoothstep(tEnd, tEnd - 2.2, abs(t));
            float w = hw * (0.28 + 0.72 * facing) * taper;
            float a = 1.0 - smoothstep(w - px, w + px, d);
            float u = clamp(d / max(w, 1e-4), 0.0, 1.0);
            vec3 core = kk * (0.62 + 0.45 * sqrt(1.0 - u * u)) * mix(0.5, 1.0, front);
            emit = emit * (1.0 - a) + core * a;
            cover = max(cover, a);
            halo += kk * exp(-max(d - w, 0.0) / gw) * (1.0 - a) * taper * mix(0.25, 0.4, front);
        }
    }
    if (cover > 0.0) { emit *= 0.84 + 0.22 * pileTexture(uv, px); }
    return vec4(emit, cover);
}

// Keep the inks saturated where pieces and glow pile up; black stays black.
vec3 shoulder(vec3 c) {
    const float knee = 0.82;
    vec3 over = knee + (1.0 - knee) * (1.0 - exp(-(c - knee) / (1.0 - knee)));
    return mix(c, over, step(knee, c));
}

vec4 carpet(void) {
    vec4 clk = texelFetch(clock, ivec2(0), 0);
    float T = clk.y;
    vec2 off = driftOffset(clk.zw, T);
    float bass = clamp(syn_BassLevel, 0.0, 1.0) * reactivity;
    float high = clamp(syn_HighLevel, 0.0, 1.0) * reactivity;
    float kick = clamp(syn_BassHits, 0.0, 1.0) * reactivity;

    vec2 uv = _uvc / zoom;
    float px = 1.0 / (RENDERSIZE.y * zoom);
    float pop = 1.0 + 0.10 * bass;
    float gw = 0.0045;

    // A slow, low-frequency sway so the drift is a flow, not a conveyor.
    vec2 sway = 0.014 * vec2(sin(uv.y * 2.1 + T * 0.13), sin(uv.x * 1.7 - T * 0.11));

    // The UV tube is fixed above the carpet: its uneven light stays put on
    // screen while the carpet moves under it.
    float lamp = 0.80 + 0.22 * _noise(_uvc * 1.3 + vec2(T * 0.05, -T * 0.03)) + 0.20 * kick;

    // Black pile with a faint violet cast where the UV catches the fibres.
    vec2 uvPile = uv + off * mix(1.0, 0.8, parallax);
    float pile = pileTexture(uvPile, px);
    vec3 col = vec3(0.008, 0.005, 0.018)
             + vec3(0.030, 0.012, 0.080) * blacklight * (0.35 + 0.65 * pile * pile);

    float haloGain = 0.50 * glow * (1.0 + 0.9 * bass);
    vec3 halo;

    vec2 uvS = uvPile + sway * 0.6;
    col += sprinkleLayer(uvS, 0.034, 3.1, 0.88 * sprinkles, T, px, high);

    vec2 uvA = uv + off * 1.0 + sway;
    vec4 a = confettiLayer(uvA, 0.085, 41.7, 0.88 * confetti, 0.11, 0.23, T, px, pop, gw, halo);
    col = col * (1.0 - a.a) + a.rgb + halo * haloGain;

    vec2 uvR = uv + off * mix(1.0, 1.35, parallax) + sway * 1.3;
    vec4 r = streamerLayer(uvR, 0.34, 77.3, 0.80 * streamers, T, px, pop, gw, halo);
    col = col * (1.0 - r.a) + r.rgb + halo * haloGain;

    vec2 uvB = uv + off * mix(1.0, 1.7, parallax) + sway * 1.6;
    vec4 b = confettiLayer(uvB, 0.18, 113.9, 0.55 * confetti, 0.10, 0.17, T, px, pop, gw, halo);
    col = col * (1.0 - b.a) + b.rgb + halo * haloGain;

    col *= lamp * brightness;
    return vec4(shoulder(col), 1.0);
}

vec4 renderMain(void) {
    if (PASSINDEX == 0) { return clockPass(); }
    return carpet();
}
