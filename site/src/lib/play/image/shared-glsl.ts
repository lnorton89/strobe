/*
 * The GLSL every From image scene shares (D560): the clock, the painter's-order
 * ink, the tone, and the marks. Every bug the hand-made scenes had to fix one
 * at a time is fixed here once: kicks only push clocks so nothing jumps, a
 * halo is never clipped because it is added by distance rather than per cell,
 * line widths never drop below a pixel, and no fwidth (its 2x2 blocks show).
 *
 * All of it is written to compile as both GLSL 330 and ES 3.00: float
 * literals with a point, no implicit int to float.
 */

/** The clock pass and the top of the final pass. */
export const CLOCK = `const float K_TAU = 6.2831853;

// Texel: x scene time last frame, y flow clock, z sway clock, w beat clock.
// Kicks push the clocks' rates and never a position, so nothing ever jumps,
// and each clock wraps so it stays exact for hours.
vec4 clockPass() {
    vec4 prev = texelFetch(clock, ivec2(0, 0), 0);
    if (prev.x <= 0.0) return vec4(TIME, TIME * speed, TIME * speed * 0.5, 0.0);
    float gap = TIME - prev.x;
    float dt = (gap > 0.0 && gap < 0.2) ? gap : 1.0 / 60.0;
    float kick = clamp(syn_BassHits, 0.0, 1.0) * reactivity;
    float mid = clamp(syn_MidLevel, 0.0, 1.0) * reactivity;
    return vec4(TIME,
                mod(prev.y + dt * speed * (1.0 + 1.4 * kick), 3600.0),
                mod(prev.z + dt * speed * (0.4 + 0.8 * mid), 3600.0),
                mod(prev.w + dt * (0.08 * speed + 2.0 * kick), 3600.0));
}
`;

/** State, noise, ink and marks, before the template's picture. */
export const PAINT = `float gFlow = 0.0;
float gSway = 0.0;
float gBeat = 0.0;
float gPx = 0.001;
float gBass = 0.0;
float gHigh = 0.0;
vec3 gCol = vec3(0.0);
vec3 gHalo = vec3(0.0);
// Fades a halo to nothing before the edge of the cell it was drawn in, so a
// neighbour's cell never cuts it off in a straight line.
float gHaloMask = 1.0;

mat2 turn(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, -s, s, c);
}

float vn(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(_hash12(i), _hash12(i + vec2(1.0, 0.0)), f.x),
               mix(_hash12(i + vec2(0.0, 1.0)), _hash12(i + vec2(1.0, 1.0)), f.x), f.y);
}

float fbm4(vec2 p) {
    float sum = 0.0;
    float amp = 0.5;
    for (int i = 0; i < 4; i++) {
        sum += amp * vn(p);
        p = mat2(1.6, 1.2, -1.2, 1.6) * p + vec2(3.1, 1.7);
        amp *= 0.5;
    }
    return sum / 0.9375;
}

// Painter's order: a mark covers what is under it and leaves its halo in a
// buffer of its own, added at the end, so a halo never shows a cell's edge.
void ink(float d, vec3 c, float halo) {
    float cover = 1.0 - smoothstep(-gPx, gPx, d);
    gCol = mix(gCol, c, cover);
    float reach = 0.012 + 0.014 * glow;
    gHalo = max(gHalo, c * exp(-max(d, 0.0) / reach) * halo * glow * gHaloMask * (0.32 + 0.3 * gBass));
}

float sdSeg(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a;
    vec2 ba = b - a;
    return length(pa - ba * clamp(dot(pa, ba) / max(dot(ba, ba), 1e-5), 0.0, 1.0));
}

float sdBox(vec2 p, vec2 b) {
    vec2 q = abs(p) - b;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0);
}

float sdDiamond(vec2 p, float r) {
    return (abs(p.x) + abs(p.y) - r) * 0.7071;
}

// A hand-made sign out of up to four strokes, chosen by id. q runs from -1 to
// 1 across its cell and every stroke stays well inside that, so a stroke and
// its halo fit the cell; the answer is in the cell's units.
float sign4(vec2 q, float id, float width) {
    float d = 1e3;
    float sym = _hash12(vec2(id, 3.7));
    if (sym < 0.55) q.x = abs(q.x);
    for (int i = 0; i < 4; i++) {
        float h = _hash12(vec2(id, float(i) * 7.3 + 1.0));
        float h2 = _hash12(vec2(float(i) * 5.1 + 2.0, id));
        float h3 = _hash12(vec2(id + 9.0, float(i) * 2.9));
        vec2 at = (vec2(floor(h2 * 3.0), floor(h3 * 3.0)) - 1.0) * 0.42;
        float s;
        if (h < 0.22) s = sdSeg(q, at + vec2(-0.3, 0.0), at + vec2(0.3, 0.0));
        else if (h < 0.44) s = sdSeg(q, at + vec2(0.0, -0.3), at + vec2(0.0, 0.3));
        else if (h < 0.58) s = sdSeg(q, at + vec2(-0.26, -0.26), at + vec2(0.26, 0.26));
        else if (h < 0.7) s = abs(length(q - at) - 0.2);
        else if (h < 0.8) s = min(sdSeg(q, at + vec2(-0.24, -0.12), at + vec2(0.0, 0.16)),
                                  sdSeg(q, at + vec2(0.24, -0.12), at + vec2(0.0, 0.16)));
        else if (h < 0.9) s = abs(sdDiamond(q - at, 0.2));
        else s = length(q - at) - 0.05;
        d = min(d, s);
        if (i >= 1 && h2 > 0.72) break;
    }
    return d - width;
}

// The ground: the colour with a slow mottle and a sheen, so it is never a
// flat fill and never competes with the marks.
vec3 groundAt(vec2 p) {
    float m = fbm4(p * 2.2 + vec2(SEED, gFlow * 0.01));
    vec3 c = ground_color * (0.82 + 0.36 * m);
    return mix(c, second_color, 0.05 + 0.07 * smoothstep(0.45, 0.8, m));
}
`;

/** The end of renderMain: halo, tone, vignette, and a dither against banding. */
export const FINISH = `vec3 toneOut(vec3 c, vec2 p) {
    c += gHalo;
    vec3 over = max(c - 0.82, 0.0);
    c = min(c, 0.82) + 0.18 * (1.0 - exp(-over / 0.18));
    c *= 1.0 - 0.32 * smoothstep(0.35, 1.1, length(p));
    c *= brightness;
    return c + (_hash12(_xy + fract(TIME)) - 0.5) / 255.0;
}

vec4 renderMain() {
    if (PASSINDEX == 0) return clockPass();
    vec4 clocks = texelFetch(clock, ivec2(0, 0), 0);
    gFlow = clocks.y;
    gSway = clocks.z;
    gBeat = clocks.w + BANG;
    gPx = 1.0 / RENDERSIZE.y;
    gBass = clamp(syn_BassLevel, 0.0, 1.0) * reactivity;
    gHigh = clamp(syn_HighLevel, 0.0, 1.0) * reactivity;
    vec3 c = picture(_uvc);
    return vec4(toneOut(c, _uvc), 1.0);
}
`;
