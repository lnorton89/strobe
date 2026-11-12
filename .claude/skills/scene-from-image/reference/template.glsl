// <Title>: Strobe interpretation of <what the artwork is>.
//
// READING. <Two to six sentences: the artwork without the room, its subject,
// composition, materials, marks and paint colours, from step 1 of the skill.>
//
// ARCHETYPE. <grid | polar | axial | path | windows | field>, recomposed for
// 16:9 by <how the sides are filled>.
//
// MOTION. <What moves and why; which clocks the clock pass integrates; what
// each band of the music does.>
//
// Two passes: a two-texel float clock integrates every rate, so a slider or
// a kick changes a speed and never makes the picture jump; then the panel is
// drawn analytically in frame-height units centred on the middle.

// The engine predefines PI, _hash12, _uvc, _xy, RENDERSIZE and TIME: do not
// redefine them.
const float TAU = 6.2831853;

// A bang (`bang counter smooth`) climbs from one whole number to the next on
// each press. This rises at once and settles over the rest of the climb, so
// a press shows within a frame: 0 at rest, 1 just after the press.
float bangPulse(float counter) {
    float f = counter - floor(counter + 0.05);
    return f > 0.0 ? 1.0 - smoothstep(0.0, 0.95, f) : 0.0;
}

float stepDt(float prevTime) {
    float d = TIME - prevTime;
    return (d > 0.0 && d < 0.2) ? d : 1.0 / 60.0;
}

float kickLevel() {
    return clamp(syn_BassHits, 0.0, 1.0) * reactivity;
}

// Example rate: radians per second at full `spin`, and a kick's push.
const float TURN_RATE = 0.3;
const float TURN_KICK = 2.0;

// Texel 0 - x: scene time last frame, y: flow time, z: turn, w: free.
// Texel 1 - free for springs (position, velocity) or more clocks.
vec4 clockPass() {
    vec4 prev0 = texelFetch(clock, ivec2(0, 0), 0);
    vec4 prev1 = texelFetch(clock, ivec2(1, 0), 0);
    float kick = kickLevel();
    // A cleared target (first frame, or a resize) restarts at the scene time.
    bool fresh = prev0.x <= 0.0;
    float dt = stepDt(prev0.x);
    if (_xy.x < 1.0) {
        if (fresh) {
            return vec4(TIME, TIME * speed, mod(TIME * speed * spin * TURN_RATE, TAU), 0.0);
        }
        float turn = speed * spin * (TURN_RATE + TURN_KICK * kick);
        return vec4(TIME, prev0.y + dt * speed, mod(prev0.z + dt * turn, TAU), 0.0);
    }
    if (fresh) { return vec4(0.0); }
    return prev1;
}

mat2 rot(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, -s, s, c);
}

// Value noise with its gradient: x the value, yz d/dx and d/dy.
vec3 vnoiseD(vec2 x) {
    vec2 i = floor(x);
    vec2 f = x - i;
    vec2 u = f * f * (3.0 - 2.0 * f);
    vec2 du = 6.0 * f * (1.0 - f);
    float a = _hash12(i);
    float b = _hash12(i + vec2(1.0, 0.0));
    float c = _hash12(i + vec2(0.0, 1.0));
    float d = _hash12(i + vec2(1.0, 1.0));
    float k = a - b - c + d;
    return vec3(a + (b - a) * u.x + (c - a) * u.y + k * u.x * u.y,
                du * vec2(b - a + k * u.y, c - a + k * u.x));
}

float vnoise(vec2 x) {
    return vnoiseD(x).x;
}

// A pen stroke that thins from half width w0 at a to w1 at b.
float sdTaper(vec2 p, vec2 a, vec2 b, float w0, float w1) {
    vec2 pa = p - a;
    vec2 ba = b - a;
    float h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-8), 0.0, 1.0);
    return length(pa - ba * h) - mix(w0, w1, h);
}

float sdBox(vec2 p, vec2 b) {
    vec2 d = abs(p) - b;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

// A four-point sparkle: broad at the heart, fine at the points.
float sparkle(vec2 q, float r) {
    vec2 a = abs(q);
    if (a.y > a.x) { a = a.yx; }
    return sdTaper(a, vec2(0.0), vec2(r, 0.0), 0.2 * r, 0.004 * r);
}

// Keep the paint's colour where glow piles up; black stays black.
vec3 shoulder(vec3 c) {
    const float knee = 0.82;
    vec3 over = knee + (1.0 - knee) * (1.0 - exp(-(c - knee) / (1.0 - knee)));
    return mix(c, over, step(knee, c));
}

// ------------------------------------------------------------------ paint

// Per-pixel state: colour so far, brightest halo, half pixel footprint,
// the coat's unevenness, the halo's width, the pen's pressure.
vec3 gCol;
vec3 gGlowC;
float gGlow;
float gPx;
float gTex;
float gGw;
float gThick;

// Past this distance from its stroke the halo is zero, so a bounds test with
// this margin can never clip it into a rectangle.
const float GLOW_REACH = 0.03;

// Lay paint of colour c over the shape d (signed distance in frame heights),
// and let it glow `g` onto what is around it. The halo lies outside the
// stroke only, so a big fill does not wash out when `glow` is high, and paint
// laid on top covers the halos beneath it.
void paint(float d, vec3 c, float g) {
    d -= gThick;
    float a = 1.0 - smoothstep(-gPx, gPx, d);
    gCol = mix(gCol, c * gTex, a);
    gGlow *= 1.0 - a;
    float e = max(d, 0.0);
    float h = g * exp(-e / gGw) * max(1.0 - e / GLOW_REACH, 0.0) * (1.0 - a);
    if (h > gGlow) { gGlow = h; gGlowC = c; }
}

// A flat coat with no halo (washes, substrate patches, shadows). It covers
// any halo beneath it.
void coat(float d, vec3 c) {
    float a = 1.0 - smoothstep(-gPx, gPx, d);
    gCol = mix(gCol, c, a);
    gGlow *= 1.0 - a;
}

// ------------------------------------------------------------------ parts

// Replace these with the artwork's parts, one function each, painter's order.
void sky(vec2 p, float T, float high) {
    for (int i = 0; i < 9; i++) {
        vec2 c = vec2(_hash12(vec2(float(i), 3.1)) * 1.7 - 0.85, _hash12(vec2(float(i), 7.7)) * 0.9 - 0.45);
        float r = 0.012 + 0.02 * _hash12(vec2(float(i), 1.3));
        float tw = 0.75 + 0.25 * sin(T * 2.0 + float(i) * 2.4) + 0.4 * high;
        if (length(p - c) < r + GLOW_REACH) { paint(sparkle(p - c, r), ink_color * tw, 0.7); }
    }
}

void hero(vec2 p, float turn, float bass) {
    vec2 q = rot(turn) * p;
    float ring = abs(length(q) - 0.3 * size) - 0.006 * weight * (1.0 + 0.3 * bass);
    float spoke = sdTaper(vec2(abs(q.x), q.y), vec2(0.0), vec2(0.2 * size, 0.2 * size), 0.008 * weight, 0.002);
    paint(min(ring, spoke), ink_color, 1.0);
}

// ------------------------------------------------------------------ panel

vec4 panel() {
    vec4 c0 = texelFetch(clock, ivec2(0, 0), 0);
    float T = c0.y;
    float turn = c0.z;
    float bass = clamp(syn_BassLevel, 0.0, 1.0) * reactivity;
    float high = clamp(syn_HighLevel, 0.0, 1.0) * reactivity;

    vec2 p0 = _uvc;
    gPx = 0.75 / RENDERSIZE.y;
    // The hand's waver: a noise gradient pushes every line a little.
    vec3 wn = vnoiseD(p0 * 17.0 + 3.1);
    vec3 wl = vnoiseD(p0 * 7.0 + 11.7);
    vec2 p = p0 + (wn.yz * 0.0016 + wl.yz * 0.0042) * wobble;

    // The substrate.
    vec3 ground = vec3(0.012, 0.011, 0.016) * (0.85 + 0.3 * vnoise(p0 * 3.0));
    gCol = ground;
    gGlow = 0.0;
    gGlowC = vec3(0.0);
    gTex = 0.86 + 0.14 * vnoise(p0 * 260.0);
    gThick = (vnoise(p0 * 48.0 + 5.3) - 0.5) * 0.003 * wobble;
    gGw = (0.0015 + 0.0018 * glow) * (1.0 + 0.5 * bass);

    sky(p, T, high);
    hero(p, turn, bass);

    vec3 col = gCol + gGlowC * gGlow * 0.3 * glow * (1.0 + bass);
    float vig = 1.0 - 0.3 * dot(p0, p0);
    return vec4(shoulder(col * vig * brightness), 1.0);
}

vec4 renderMain(void) {
    if (PASSINDEX == 0) { return clockPass(); }
    return panel();
}
