// Jester Totem: original procedural interpretation of a hand-painted
// blacklight panel. A horned, one-eyed jester in a striped party hat rides a
// lightning bolt, a die and a starburst on the coiled spring of a cranked
// jack-in-the-box, painted in fluorescent yellow, cyan, pink and orange on a
// dark plank, with sparkles and crosses around it.
//
// Two passes: a one-texel clock integrates the flow time and a damped spring
// (position and velocity), so bass and kicks push the jack up and it
// overshoots, bounces and settles like a real one, at any frame rate. Then
// the panel is drawn analytically: planks under a fan of UV rays that turns
// behind the jester and flares on kicks, a drifting field of motifs on the
// neighbouring planks, the fixed ornaments, the box, the spring, and the
// jack's group (starburst, die, bolt, head) lifted, tilted, and stretched or
// squashed by the spring's speed. Twins, smaller and mirrored, can ride the
// next planks out. Every shape is a signed distance with a smoothstep over
// the pixel footprint, and each group is skipped outside its bounding box.
//
// Live: `pop` launches the jester on demand; `twins` fades the side totems
// in and out; `rays`, `hue_cycle` and `kick_flash` set how loud it is.

const float TAU = 6.2831853;

// The panel is two units tall; this fits it to the screen height with a
// small margin.
const float FRAME = 2.0 / 0.94;
const float PLANK = 0.94;

const vec3 YELLOW = vec3(1.00, 0.90, 0.10);
const vec3 LIME   = vec3(0.80, 1.00, 0.22);
const vec3 CYAN   = vec3(0.28, 0.95, 1.00);
const vec3 PINK   = vec3(1.00, 0.40, 0.70);
const vec3 ORANGE = vec3(1.00, 0.52, 0.08);
const vec3 EMBER  = vec3(1.00, 0.28, 0.06);
const vec3 CREAM  = vec3(1.00, 0.95, 0.60);
const vec3 BOLT   = vec3(0.86, 1.00, 1.00);
const vec3 DARK   = vec3(0.020, 0.012, 0.018);

// Spring of the jack: natural frequency (Hz), damping ratio and travel.
const float SPRING_HZ = 1.7;
const float SPRING_ZETA = 0.16;
const float JACK_MIN = -0.05;
const float JACK_MAX = 0.16;

// The box's die, whose top face the spring rises from.
const vec2 BOX_C = vec2(0.04, -0.71);
const float BOX_S = 0.36;
const vec2 SPRING_BASE = vec2(0.04, -0.5696);   // BOX_C + BOX_S * (0, 0.39)
const vec2 BURST = vec2(0.03, -0.38);
const vec2 DIE_C = vec2(0.03, -0.15);
const float DIE_S = 0.28;
const vec2 HEAD = vec2(0.02, 0.45);
const float HEAD_R = 0.22;

// Per-pixel state shared by the drawing helpers.
vec3 gCol;
vec3 gGlo;
float gPx;
float gTex;

float flowRate() {
    return speed * (1.0 + 0.6 * clamp(syn_MidLevel, 0.0, 1.0) * reactivity);
}

float stepDt(float prevTime) {
    float d = TIME - prevTime;
    return (d > 0.0 && d < 0.2) ? d : 1.0 / 60.0;
}

// Where the spring wants to be: a slow idle bob, held up by the bass and
// thrown up by kicks and by the pop button. The spring chases it, so a kick
// overshoots.
float springTarget(float T) {
    float bass = clamp(syn_BassLevel, 0.0, 1.0) * reactivity;
    float kick = clamp(syn_BassHits, 0.0, 1.0) * reactivity;
    return bounce * (0.012 * sin(T * 1.3) + 0.03 * bass + 0.10 * kick)
         + 0.16 * clamp(pop, 0.0, 1.0);
}

// x: scene time last frame, y: flow time, z: jack lift, w: jack velocity.
vec4 clockPass() {
    vec4 prev = texelFetch(clock, ivec2(0), 0);
    float rate = flowRate();
    // A cleared target (first frame, or a resize) restarts at rest.
    if (prev.x <= 0.0) { return vec4(TIME, TIME * rate, 0.0, 0.0); }
    float dt = stepDt(prev.x);
    float T = prev.y + dt * rate;
    float target = springTarget(T);
    float w = TAU * SPRING_HZ;
    float x = prev.z;
    float v = prev.w;
    // Two semi-implicit Euler substeps: stable well past 60 fps' step.
    float h = 0.5 * dt;
    for (int i = 0; i < 2; i++) {
        v += (-w * w * (x - target) - 2.0 * SPRING_ZETA * w * v) * h;
        x += v * h;
    }
    // Hitting the end of its travel stops it, as the box's lid would.
    if (x < JACK_MIN || x > JACK_MAX) { v = 0.0; }
    return vec4(TIME, T, clamp(x, JACK_MIN, JACK_MAX), clamp(v, -3.0, 3.0));
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

float sdSegment(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a;
    vec2 ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// A stroke from a to b whose half width goes from ra to rb.
float sdTaper(vec2 p, vec2 a, vec2 b, float ra, float rb) {
    vec2 pa = p - a;
    vec2 ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h) - mix(ra, rb, h);
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

// A four-point sparkle of radius r: two thin crossed diamonds.
float sdSparkle(vec2 p, float r) {
    return min(sdRhombus(p, vec2(0.2 * r, r)), sdRhombus(p, vec2(r, 0.2 * r)));
}

// An eight-point star: a sparkle over a smaller one turned 45 degrees.
float sdStar8(vec2 p, float r) {
    return min(sdSparkle(p, r), sdSparkle(rot(0.7853982) * p, 0.6 * r));
}

// The painted crosses: a long upright, a crossbar and a short lower bar.
float sdCross(vec2 p, float s) {
    p /= s;
    float d = sdBox(p - vec2(0.0, -0.02), vec2(0.011, 0.11));
    d = min(d, sdBox(p - vec2(0.0, 0.035), vec2(0.06, 0.011)));
    d = min(d, sdBox(p - vec2(0.0, -0.075), vec2(0.03, 0.009)));
    return d * s;
}

float coverage(float d) {
    return 1.0 - smoothstep(-gPx, gPx, d);
}

// Lay paint of colour c over the shape, and let it glow onto what is
// around it. Under UV, fluorescent paint bleeds a soft halo.
void paintFill(float d, vec3 c) {
    float a = coverage(d);
    gCol = mix(gCol, c * gTex, a);
    gGlo += c * exp(-max(d, 0.0) * 70.0) * (1.0 - a);
}

// Paint without a halo: dark mouths, pips and the like.
void paintFlat(float d, vec3 c) {
    gCol = mix(gCol, c, coverage(d));
}

// The dark brush outline the panel's shapes are drawn with.
void paintLine(float d, float w) {
    gCol = mix(gCol, DARK, coverage(abs(d) - w));
}

vec3 paletteAt(float k) {
    if (k < 1.0) { return YELLOW; }
    if (k < 2.0) { return CYAN; }
    if (k < 3.0) { return PINK; }
    if (k < 4.0) { return ORANGE; }
    return LIME;
}

// Dark planks with a vertical grain, a groove between them, and a faint
// violet cast where the UV catches the fibres. Returns the grain in `grainOut`
// so the paint can pick it up.
vec3 wood(vec2 p, out float grainOut) {
    float k = floor(p.x / PLANK + 0.5);
    float lx = p.x - k * PLANK;
    float warp = _noise(vec2(p.y * 0.9 + k * 5.1, lx * 2.0 + k * 3.7));
    float g = _noise(vec2(lx * min(34.0, 0.2 / gPx) + 4.0 * warp, p.y * 0.8 + k * 11.3));
    grainOut = g;
    float tint = _hash11(k * 3.7 + 1.3);
    vec3 brown = mix(vec3(0.050, 0.030, 0.022), vec3(0.040, 0.030, 0.030), tint);
    vec3 col = brown * mix(0.75, 0.45 + 0.9 * g, grain)
             + vec3(0.030, 0.012, 0.080) * blacklight * (0.4 + 0.6 * g);
    float groove = 1.0 - smoothstep(0.004, 0.004 + 2.0 * gPx, 0.5 * PLANK - abs(lx));
    return col * (1.0 - 0.85 * groove);
}

// Motifs on the neighbouring planks, rising slowly: sparkles, crosses,
// eight-point stars and dots on a jittered grid, one cell per pixel. The
// planks within `clear` of the middle (the totem's, and the twins') stay
// clear.
void fieldLayer(vec2 p, float cell, float seed, float dens, float rise, float T, float high, float clear) {
    vec2 q = vec2(p.x, p.y - rise) / cell;
    vec2 id = floor(q);
    if (abs((id.x + 0.5) * cell) < clear + 0.5 * cell) { return; }
    vec4 h = _hash42(id + seed);
    if (h.x > dens) { return; }
    vec4 g = _hash42(id * 1.37 + seed + 7.3);
    vec2 f = (q - id - 0.5 - (g.xy - 0.5) * 0.2) * cell;
    float tw = 0.65 + 0.35 * sin(T * (0.8 + 1.6 * g.w) + g.z * TAU);
    tw += 1.2 * high * step(0.7, g.x) * (0.5 + 0.5 * sin(T * 6.0 + g.y * TAU));
    float r = mix(0.22, 0.32, h.w) * cell * (0.9 + 0.1 * tw);
    f = rot(g.z * TAU + T * (g.w - 0.5) * 0.5 * spin) * f;
    float type = floor(h.y * 4.0);
    float d;
    if (type < 1.0) {
        d = sdSparkle(f, r);
    } else if (type < 2.0) {
        d = sdCross(f, r * 8.0);
    } else if (type < 3.0) {
        d = sdStar8(f, r);
    } else {
        d = length(f - vec2(0.0, 0.35 * r)) - 0.16 * r;
        d = min(d, length(f - vec2(-0.32 * r, -0.2 * r)) - 0.16 * r);
        d = min(d, length(f - vec2(0.32 * r, -0.2 * r)) - 0.16 * r);
    }
    // A step back from the totem: dimmer than its paint.
    paintFill(d, paletteAt(floor(h.z * 5.0)) * (0.6 * tw));
}

// A fan of soft UV rays from `o` across the whole panel, turning slowly and
// flaring on kicks: alternate wedges in the four inks, fading with distance
// so the planks far out stay dark. Laid on the wood, under everything.
void raysLayer(vec2 p, vec2 o, float T, float kick, float amount) {
    vec2 q = p - o;
    float r = length(q);
    const float N = 18.0;
    float u = (atan(q.y, q.x) + T * 0.08 * spin) / TAU * N;
    float k = floor(u);
    float wedge = 1.0 - smoothstep(0.12, 0.42, abs(fract(u) - 0.5));
    float fade = exp(-0.35 * r) * smoothstep(0.05, 0.35, r);
    vec3 c = paletteAt(mod(k, 4.0));
    gCol += c * wedge * fade * amount * (0.24 + 0.40 * kick);
}

// The fixed decoration around the totem.
void ornaments(vec2 p, float T, float high) {
    float tw = 1.0 + 0.12 * sin(T * 1.7) + 0.35 * high;
    vec2 m = vec2(abs(p.x), p.y);
    // Upper sparkles and pink dots.
    if (p.y > 0.64) {
        paintFill(sdSparkle(m - vec2(0.345, 0.87), 0.075 * tw), YELLOW);
        paintFill(length(m - vec2(0.23, 0.875)) - 0.014, PINK);
    }
    // Crosses beside the head.
    if (abs(p.y - 0.30) < 0.2) {
        paintFill(sdCross(m - vec2(0.315, 0.30), 1.0), ORANGE);
    }
    // Small sparkles and their dots beside the die.
    if (abs(p.y - 0.03) < 0.16) {
        vec2 q = m - vec2(0.31, 0.03);
        paintFill(sdSparkle(q, 0.048 * tw), ORANGE);
        vec2 a = abs(q);
        paintFill(min(length(a - vec2(0.06, 0.035)), length(a - vec2(0.035, 0.06))) - 0.009, ORANGE);
    }
    // Pink stars at the foot.
    if (p.y < -0.72) {
        vec2 q = vec2(abs(p.x - 0.02), p.y) - vec2(0.32, -0.90);
        q = rot(T * 0.2 * spin) * q;
        paintFill(sdStar8(q, 0.07 * tw), PINK);
        paintFill(sdSparkle(q, 0.028), CYAN);
    }
}

// Signed distance to the parallelogram o + u a + v b (u, v in [0, 1]),
// and the point's (u, v).
float sdPara(vec2 p, vec2 o, vec2 a, vec2 b, out vec2 uv) {
    vec2 q = p - o;
    float det = a.x * b.y - a.y * b.x;
    uv = vec2(q.x * b.y - q.y * b.x, a.x * q.y - a.y * q.x) / det;
    float du = (abs(uv.x - 0.5) - 0.5) * abs(det) / length(b);
    float dv = (abs(uv.y - 0.5) - 0.5) * abs(det) / length(a);
    return max(du, dv);
}

// Whether a die face showing `v` has a pip in cell g of its 3x3 grid.
float pipOn(float v, vec2 g) {
    if (g.x == 1.0 && g.y == 1.0) { return mod(v, 2.0); }
    if (g.x == 1.0) { return 0.0; }
    if (g.y == 1.0) { return step(5.5, v); }
    if (g.x == g.y) { return step(1.5, v); }
    return step(3.5, v);
}

void dieFace(vec2 p, vec2 o, vec2 a, vec2 b, float v, vec3 c) {
    vec2 uv;
    float d = sdPara(p, o, a, b, uv);
    if (d > 0.06) { return; }
    paintFill(d, c);
    vec2 g = clamp(floor(uv * 3.0), 0.0, 2.0);
    vec2 e = uv - (0.22 + 0.28 * g);
    float pd = length(e.x * a + e.y * b) - 0.075 * min(length(a), length(b));
    if (pipOn(v, g) > 0.5 && d < 0.0) { paintFlat(max(pd, d), DARK); }
    paintLine(d, 0.0045);
}

// An oblique cube: the projections of its right, up and depth axes.
const vec2 AX = vec2(0.70, -0.16);
const vec2 AY = vec2(0.00, 0.78);
const vec2 AZ = vec2(0.38, 0.22);

// A die centred on c with edge s showing v on its front face. The other two
// faces show values that sit next to it on a real die.
void drawDie(vec2 p, vec2 c, float s, float v, vec3 ink) {
    vec2 o = c - s * vec2(0.54, 0.42);
    float vr = mod(v + 1.0, 6.0) + 1.0;
    if (vr + v == 7.0) { vr = mod(vr, 6.0) + 1.0; }
    float vt = mod(v + 3.0, 6.0) + 1.0;
    if (vt + v == 7.0 || vt + vr == 7.0 || vt == vr) { vt = mod(vt + 1.0, 6.0) + 1.0; }
    dieFace(p, o, AX * s, AY * s, v, ink * 0.92);
    dieFace(p, o + AX * s, AZ * s, AY * s, vr, ink * 0.68);
    dieFace(p, o + AY * s, AX * s, AZ * s, vt, mix(ink, YELLOW, 0.3));
}

// The crank on the box's front face, turning with the music's flow.
void drawCrank(vec2 p, float T) {
    vec2 o = BOX_C - BOX_S * vec2(0.54, 0.42);
    vec2 axle = o + BOX_S * (0.08 * AX + 0.55 * AY);
    vec2 elbow = axle + vec2(-0.10, 0.0);
    float w = T * 2.2 * spin;
    vec2 grip = elbow + 0.06 * vec2(cos(w), 0.7 * sin(w));
    float d = min(sdSegment(p, axle, elbow), sdSegment(p, elbow, grip)) - 0.011;
    paintFill(d, EMBER);
    paintLine(d, 0.0035);
    // The knob is a coil that turns with the handle.
    vec2 k = rot(-w) * (p - grip);
    float r = length(k);
    float turns = r / 0.011 - atan(k.y, k.x) / TAU;
    float sp = max((abs(fract(turns) - 0.5) - 0.24) * 0.011, r - 0.034);
    paintFill(sp, EMBER);
}

// The coil from the box to the starburst: rings stacked along the axis,
// widening upward, each tilted a little so they read as one helix. Back
// halves first, dimmer; front halves over them.
void drawSpring(vec2 p, vec2 top) {
    vec2 axis = top - SPRING_BASE;
    float L = length(axis);
    vec2 u = axis / L;
    vec2 q = p - SPRING_BASE;
    vec2 lp = vec2(dot(q, vec2(u.y, -u.x)), dot(q, u));
    if (abs(lp.x) > 0.13 || lp.y < -0.06 || lp.y > L + 0.06) { return; }
    const float RINGS = 6.0;
    for (int k = 0; k < 12; k++) {
        float i = mod(float(k), RINGS);
        float front = k < 6 ? 0.0 : 1.0;
        float f = (i + 0.5) / RINGS;
        float rx = mix(0.032, 0.068, f);
        float ry = 0.30 * rx;
        vec2 c = rot(0.12) * (lp - vec2(0.0, L * f));
        float th = atan(c.y / ry, c.x / rx);
        if ((sin(th) < 0.0) != (front > 0.5)) { continue; }
        float d = length(c - vec2(rx * cos(th), ry * sin(th))) - 0.0065;
        paintFill(d, CYAN * mix(0.55, 1.0, front));
    }
}

void drawStarburst(vec2 p, float T, float kick) {
    vec2 q = p - BURST;
    if (dot(q, q) > 0.42 * 0.42) { return; }
    float per = TAU / 8.0;
    float a = atan(q.y, q.x);
    float ai = floor(a / per + 0.5);
    float m = mod(ai, 4.0);
    float len = (m < 0.5 ? 0.27 : (m > 1.5 && m < 2.5 ? 0.12 : 0.20)) * (1.0 + 0.3 * kick);
    vec2 s = rot(ai * per) * q;
    float d = sdTaper(s, vec2(0.0), vec2(len, 0.0), 0.011, 0.002);
    paintFill(d, ORANGE);
    vec2 mq = vec2(abs(q.x), q.y);
    paintFill(sdCross(mq - vec2(0.245, 0.0), 0.45), ORANGE);
    paintFill(length(vec2(mq.x, abs(q.y)) - vec2(0.17, 0.055)) - 0.01, ORANGE);
    paintFill(sdSparkle(rot(T * 0.5 * spin) * q, 0.06 * (1.0 + 0.4 * kick)), CYAN);
    paintFill(length(q) - 0.012, BOLT);
}

// The bolt from the die up into the jester's collar, re-zagged with the highs.
void drawBolt(vec2 p, float T, float high) {
    if (lightning <= 0.0 || abs(p.x - 0.025) > 0.09 || p.y < -0.05 || p.y > 0.20) { return; }
    vec2 a = vec2(0.03, 0.02);
    vec2 b = vec2(0.02, 0.15);
    float step12 = floor(T * 12.0);
    float d = 1e3;
    vec2 prev = a;
    for (int i = 1; i <= 6; i++) {
        float fi = float(i);
        float side = mod(fi, 2.0) * 2.0 - 1.0;
        float jit = 1.0 + 0.8 * high * (_hash11(step12 + fi * 7.1) - 0.5);
        vec2 pt = mix(a, b, fi / 6.0) + vec2(i == 6 ? 0.0 : side * 0.022 * jit, 0.0);
        d = min(d, sdSegment(p, prev, pt));
        prev = pt;
    }
    float flick = lightning * (0.8 + 1.4 * high);
    paintFill(d - 0.010, YELLOW * min(flick, 1.6));
    paintFill(d - 0.0045, BOLT * min(flick, 1.6));
}

// One horn, for the left side; the right is its mirror. A tapering arc that
// curls up and inward, with cyan bands.
void drawHorn(vec2 hp, float ang) {
    const vec2 C = vec2(-0.24, 0.22);
    const float R = 0.12;
    const float TH0 = -1.0;
    const float SWEEP = 4.4;
    vec2 attach = C + R * vec2(cos(TH0), sin(TH0));
    vec2 q = rot(ang) * (hp - attach) + attach - C;
    if (dot(q, q) > 0.22 * 0.22) { return; }
    float phi = mod(TH0 - atan(q.y, q.x), TAU);
    float t = phi / SWEEP;
    float d;
    if (t <= 1.0) {
        d = abs(length(q) - R) - mix(0.058, 0.009, t);
    } else {
        float te = TH0 - SWEEP;
        d = min(length(q - R * vec2(cos(TH0), sin(TH0))) - 0.058,
                length(q - R * vec2(cos(te), sin(te))) - 0.009);
    }
    float band = (abs(fract(t * 7.0 + 0.25) - 0.5) - 0.2) * SWEEP * R / 7.0;
    vec3 c = mix(LIME, CYAN, coverage(band) * step(t, 1.0));
    paintFill(d, c);
    paintLine(d, 0.004);
}

void drawHat(vec2 hp, float T) {
    if (abs(hp.x) > 0.13 || hp.y < 0.15 || hp.y > 0.40) { return; }
    float tilt = wobble * 0.12 * sin(T * 1.3 + 0.5);
    vec2 q = rot(tilt) * (hp - vec2(0.0, 0.20));
    float d = sdTri(vec2(q.x, 0.16 - q.y), vec2(0.085, 0.16));
    float u = (q.y + 0.5 * q.x) * 16.0 - T * 0.5 * spin;
    float band = (abs(fract(u) - 0.5) - 0.25) / 16.0;
    paintFill(d, mix(YELLOW, CYAN, coverage(band)));
    paintLine(d, 0.004);
}

void drawSunburst(vec2 hp, float T, float kick) {
    vec2 q = hp - vec2(0.0, 0.45);
    if (dot(q, q) > 0.15 * 0.15) { return; }
    q = rot(T * 0.35 * spin) * q;
    float per = TAU / 12.0;
    float ai = floor(atan(q.y, q.x) / per + 0.5);
    vec2 s = rot(ai * per) * q;
    float len = (mod(ai, 2.0) < 0.5 ? 0.10 : 0.075) * (1.0 + 0.25 * kick);
    paintFill(sdTaper(s, vec2(0.045, 0.0), vec2(len, 0.0), 0.007, 0.0015), YELLOW);
    paintFill(length(q) - 0.036, YELLOW);
    paintFill(length(q) - 0.024, PINK);
}

// The grin: a dark band across the lower face with interlocking teeth, and
// the jaw drops with the bass.
void drawMouth(vec2 hp, float bass) {
    float r = length(hp);
    float a = atan(hp.x, -hp.y);
    float band = max(abs(r - 0.165) - 0.04, (abs(a) - 1.15) * r);
    if (band > 0.02) { return; }
    paintFlat(band, DARK);
    float s = a / (2.3 / 11.0);
    float open = 0.004 + 0.02 * bass;
    float triU = abs(fract(s) - 0.5) * 2.0;
    float triL = abs(fract(s + 0.5) - 0.5) * 2.0;
    float dU = max((r - (0.125 + 0.05 * (1.0 - triU) - open)) * 0.7, band);
    float dL = max(((0.205 - 0.05 * (1.0 - triL) + open) - r) * 0.7, band);
    paintFill(dU, mix(CREAM, PINK, smoothstep(0.55, 1.0, 1.0 - triU)));
    paintFill(dL, mix(CREAM, PINK, smoothstep(0.55, 1.0, 1.0 - triL)));
    paintLine(band, 0.003);
}

// The jester's head, in its own frame centred on the face.
void drawHead(vec2 hp, float T, float bass, float mid, float kick) {
    drawSunburst(hp, T, kick);
    float side = hp.x < 0.0 ? -1.0 : 1.0;
    float hornAng = wobble * (0.12 * sin(T * 1.1 + side * 1.3) + 0.10 * mid) * side;
    drawHorn(vec2(-abs(hp.x), hp.y), hornAng);
    drawHat(hp, T);

    float r = length(hp);
    float a = atan(hp.x, -hp.y);
    // A ring of cyan dots, then the collar of pink spikes.
    if (r < 0.44 && hp.y < 0.06) {
        float di = clamp(floor(a / 0.24 + 0.5), -3.0, 3.0);
        vec2 dp = 0.355 * vec2(sin(di * 0.24), -cos(di * 0.24));
        paintFill(length(hp - dp) - 0.016, CYAN);
        float per = 2.5 / 8.0;
        float ci = clamp(floor(a / per + 0.5), -4.0, 4.0);
        vec2 sp = rot(ci * per) * hp;
        float d = sdTri(vec2(sp.x, sp.y + 0.31), vec2(0.034, 0.11));
        paintFill(d, PINK);
        paintLine(d, 0.004);
    }

    float face = r - HEAD_R;
    paintFill(face, YELLOW * (0.82 + 0.25 * smoothstep(0.08, HEAD_R, r)));
    paintLine(face, 0.005);
    if (face > 0.02) { return; }

    drawMouth(hp, bass);

    // Star eyes, spinning in opposite directions.
    vec2 ep = vec2(abs(hp.x), hp.y) - vec2(0.11, 0.075);
    ep = rot(T * 0.8 * spin * side) * ep;
    float per = TAU / 8.0;
    float ai = floor(atan(ep.y, ep.x) / per + 0.5);
    vec2 rp = rot(ai * per) * ep;
    paintFlat(sdTaper(rp, vec2(0.0), vec2(0.06, 0.0), 0.0045, 0.0015), DARK);
    paintFill(length(ep) - 0.013, PINK);

    // The nose: a pink ring around a dark hole with a pink crescent.
    vec2 np = hp - vec2(0.01, -0.02);
    paintFill(length(np) - 0.045, PINK);
    paintFlat(length(np) - 0.029, DARK);
    paintFill(max(length(np - vec2(0.0, -0.006)) - 0.02, 0.02 - length(np - vec2(0.0, 0.004))), PINK);

    // The third eye: an almond that blinks every few seconds and looks around.
    float blink = fract(T * 0.19 + 0.3);
    float open = clamp(abs(blink - 0.96) / 0.025, 0.0, 1.0);
    float w = 0.048;
    float h = max(0.022 * open, 0.002);
    float R = (w * w + h * h) / (2.0 * h);
    vec2 tp = hp - vec2(0.0, 0.155);
    float almond = max(length(tp + vec2(0.0, R - h)) - R, length(tp - vec2(0.0, R - h)) - R);
    paintFill(almond, CREAM);
    vec2 look = vec2(0.022 * (_noise(T * 0.4) - 0.5) * 2.0, 0.004 * (_noise(T * 0.3 + 7.0) - 0.5));
    paintFlat(max(length(tp - look) - 0.012, almond), DARK);
    paintLine(almond, 0.004);
}

// Keep the inks saturated where shapes and glow pile up; black stays black.
vec3 shoulder(vec3 c) {
    const float knee = 0.82;
    vec3 over = knee + (1.0 - knee) * (1.0 - exp(-(c - knee) / (1.0 - knee)));
    return mix(c, over, step(knee, c));
}

// Rotate hue about the grey axis; cheap, and black stays black.
vec3 hueShift(vec3 c, float a) {
    const vec3 k = vec3(0.57735);
    float ca = cos(a);
    return max(c * ca + cross(k, c) * sin(a) + k * dot(k, c) * (1.0 - ca), 0.0);
}

// How much a twin is smaller than the totem, and where it stands on its
// plank (its own frame's origin, in the panel's units).
const float TWIN_SCALE = 0.62;
const vec2 TWIN_AT = vec2(PLANK, -0.16);

// The box, the spring and the jack's group, in the totem's own frame (`p`
// with the totem's plank centred at x = 0). The group rides the spring:
// lifted by it, tilted about its base by the sway and by how fast it is
// moving, and stretched as it shoots up and squashed as it drops, about the
// starburst so the spring stays attached.
void drawTotem(vec2 p, float T, float jack, float jackV, vec4 audio) {
    float bass = audio.x;
    float mid = audio.y;
    float high = audio.z;
    float kick = audio.w;
    if (p.y < -0.40) {
        float roll = floor(T / 4.0);
        drawDie(p, BOX_C, BOX_S, floor(_hash11(roll * 1.7 + 3.0) * 6.0) + 1.0, EMBER);
        drawCrank(p, T);
    }

    float tilt = wobble * (0.035 * sin(T * 0.63) + 0.05 * mid * sin(T * 1.7))
               + 0.08 * clamp(jackV, -1.0, 1.0) * wobble;
    mat2 tiltM = rot(tilt);
    vec2 pj = tiltM * (p - SPRING_BASE) + SPRING_BASE - vec2(0.0, jack);
    float stretch = 1.0 + 0.2 * bounce * clamp(jackV, -1.2, 1.2);
    pj = BURST + (pj - BURST) * vec2(sqrt(stretch), 1.0 / stretch);
    // The starburst's centre in world space, for the spring's top.
    vec2 top = SPRING_BASE + (BURST + vec2(0.0, jack) - SPRING_BASE) * tiltM;
    drawSpring(p, top);

    if (pj.y < 0.22 && pj.y > -0.82) {
        drawStarburst(pj, T, kick);
        drawBolt(pj, T, high);
        float roll = floor(T / 4.0 + 0.5);
        drawDie(pj, DIE_C, DIE_S, floor(_hash11(roll * 2.3 + 11.0) * 6.0) + 1.0, ORANGE);
    }

    if (pj.y > -0.05) {
        float headTilt = wobble * (0.08 * sin(T * 0.9) + 0.10 * mid * sin(T * 2.3 + 1.0));
        vec2 neck = HEAD - vec2(0.0, 0.25);
        vec2 hp = rot(headTilt) * (pj - neck) + neck - HEAD;
        drawHead(hp, T, bass, mid, kick);
    }
}

vec4 panel(void) {
    vec4 clk = texelFetch(clock, ivec2(0), 0);
    float T = clk.y;
    float jack = clk.z;
    float jackV = clk.w;
    float bass = clamp(syn_BassLevel, 0.0, 1.0) * reactivity;
    float mid = clamp(syn_MidLevel, 0.0, 1.0) * reactivity;
    float high = clamp(syn_HighLevel, 0.0, 1.0) * reactivity;
    float kick = clamp(syn_BassHits, 0.0, 1.0) * reactivity;
    vec4 audio = vec4(bass, mid, high, kick);

    float z = zoom * (1.0 + 0.02 * kick);
    vec2 p = _uvc * FRAME / z;
    gPx = FRAME / (RENDERSIZE.y * z);

    float g;
    gCol = wood(p, g);
    gGlo = vec3(0.0);
    // Brushed paint: uneven, with the grain showing through a little.
    gTex = 0.90 + 0.12 * _noise(p * min(90.0, 0.3 / gPx)) + 0.10 * (g - 0.5) * grain;

    if (rays > 0.0) { raysLayer(p, HEAD + vec2(0.0, jack), T, kick, rays); }

    bool twinsOn = twins > 0.01;
    if (sparkles > 0.0) {
        float rise = T * 0.05;
        float clear = (twinsOn ? 1.5 : 0.5) * PLANK;
        fieldLayer(p, 0.30, 5.1, 0.55 * sparkles, rise * 0.6, T, high, clear);
        fieldLayer(p, 0.19, 9.7, 0.45 * sparkles, rise, T, high, clear);
    }

    if (abs(p.x) < 0.5 * PLANK) {
        ornaments(p, T, high);
        drawTotem(p, T, jack, jackV, audio);
    } else if (twinsOn && abs(p.x) < 1.5 * PLANK) {
        // A twin on each next plank, mirrored to face the middle, swaying
        // out of step with the totem, faded in and out by `twins`.
        float side = sign(p.x);
        vec2 tp = (vec2(abs(p.x), p.y) - TWIN_AT) / TWIN_SCALE;
        tp.x = -tp.x;
        vec3 col0 = gCol;
        vec3 glo0 = gGlo;
        float px0 = gPx;
        gPx /= TWIN_SCALE;
        drawTotem(tp, T + 2.1 + 1.3 * side, jack, jackV, audio);
        gPx = px0;
        gCol = mix(col0, gCol, twins);
        gGlo = mix(glo0, gGlo, twins);
    }

    float lamp = 0.82 + 0.2 * _noise(_uvc * 1.3 + vec2(T * 0.05, -T * 0.03))
               + 0.18 * kick * kick_flash;
    vec3 col = gCol + gGlo * 0.35 * glow * (1.0 + 0.9 * bass);
    float turn = hue + hue_cycle * T * 0.03;
    if (turn > 0.0) { col = hueShift(col, turn * TAU); }
    col *= lamp * brightness;
    return vec4(shoulder(col), 1.0);
}

vec4 renderMain(void) {
    if (PASSINDEX == 0) { return clockPass(); }
    return panel();
}
