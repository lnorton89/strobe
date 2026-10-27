// ============================================================================
// SSF function library (Strobe implementation).
//
// Compiles as desktop GLSL 3.30 core and as GLSL ES 3.00. Every float literal
// carries a decimal point, no built-in is overloaded, and every loop bound is
// constant. Private helpers are prefixed strobe_. Depends on uniforms and
// globals the assembler declares first: RENDERSIZE, _xy, _uv, syn_Media,
// syn_MediaType.
//
// Where Synesthesia's documentation and what its engine computes disagree,
// the engine wins (D491): real scenes are tuned against what they see, e.g.
// `_rotate` takes radians although the documentation says degrees.
// ============================================================================

// ---------------------------------------------------------------- math

float _scale(float value, float lo, float hi) { return lo + value * (hi - lo); }

float _map(float v, float lo, float hi, float nlo, float nhi) { return nlo + (v - lo) * (nhi - nlo) / (hi - lo); }
vec2 _map(vec2 v, vec2 lo, vec2 hi, vec2 nlo, vec2 nhi) { return nlo + (v - lo) * (nhi - nlo) / (hi - lo); }
vec3 _map(vec3 v, vec3 lo, vec3 hi, vec3 nlo, vec3 nhi) { return nlo + (v - lo) * (nhi - nlo) / (hi - lo); }
vec4 _map(vec4 v, vec4 lo, vec4 hi, vec4 nlo, vec4 nhi) { return nlo + (v - lo) * (nhi - nlo) / (hi - lo); }

// Exponential smooth minimum: larger k is a sharper join.
float _smin(float a, float b, float k) {
    float kk = max(k, 0.0001);
    return -log(exp(-kk * a) + exp(-kk * b)) / kk;
}

// The same seeds give the same values as Synesthesia's (D491), so a scene that
// picks a colour or a cell from `_rand` picks the one it was designed with.
float _rand(float seed) { return fract(sin(seed) * 43758.5453123); }
float _rand(vec2 seed) { return fract(sin(dot(seed, vec2(12.9898, 4.1414))) * 43758.5453); }

// `size` is the half-width: the pulse is 1 at `center` and 0 from
// `center ± size` outwards (D491).
float _pulse(float position, float center, float size) {
    return 1.0 - smoothstep(0.0, max(size, 0.00001), abs(position - center));
}
float _sqPulse(float position, float center, float size) {
    return step(center - size, position) - step(center + size, position);
}

// Inside means `lo <= v < hi` (D491).
float _inRange(float v, float lo, float hi) { return step(lo, v) - step(hi, v); }
float _inRange(float v) { return _inRange(v, 0.0, 1.0); }
float _inRange(vec2 v, float lo, float hi) { return _inRange(v.x, lo, hi) * _inRange(v.y, lo, hi); }
float _inRange(vec2 v) { return _inRange(v, 0.0, 1.0); }
float _inRange(vec3 v, float lo, float hi) { return _inRange(v.xy, lo, hi) * _inRange(v.z, lo, hi); }
float _inRange(vec3 v) { return _inRange(v, 0.0, 1.0); }
float _inRange(vec4 v, float lo, float hi) { return _inRange(v.xyz, lo, hi) * _inRange(v.w, lo, hi); }
float _inRange(vec4 v) { return _inRange(v, 0.0, 1.0); }

// Ranges from -0.5 to 0.0, as documented: -0.5 at each whole period, 0.0
// half way (D491).
float _triWave(float position, float period) {
    return -abs(fract(position / period) - 0.5);
}

float _pixelate(float v, float amount) { return floor(v * amount) / amount; }
vec2 _pixelate(vec2 v, float amount) { return floor(v * amount) / amount; }
vec3 _pixelate(vec3 v, float amount) { return floor(v * amount) / amount; }

float strobe_mixWeight(float m, float segments, float index) {
    return clamp(m * segments - index, 0.0, 1.0);
}
float _mix3(float a, float b, float c, float m) { return mix(mix(a, b, strobe_mixWeight(m, 2.0, 0.0)), c, strobe_mixWeight(m, 2.0, 1.0)); }
vec2 _mix3(vec2 a, vec2 b, vec2 c, float m) { return mix(mix(a, b, strobe_mixWeight(m, 2.0, 0.0)), c, strobe_mixWeight(m, 2.0, 1.0)); }
vec3 _mix3(vec3 a, vec3 b, vec3 c, float m) { return mix(mix(a, b, strobe_mixWeight(m, 2.0, 0.0)), c, strobe_mixWeight(m, 2.0, 1.0)); }
vec4 _mix3(vec4 a, vec4 b, vec4 c, float m) { return mix(mix(a, b, strobe_mixWeight(m, 2.0, 0.0)), c, strobe_mixWeight(m, 2.0, 1.0)); }
float _mix4(float a, float b, float c, float d, float m) { return mix(mix(mix(a, b, strobe_mixWeight(m, 3.0, 0.0)), c, strobe_mixWeight(m, 3.0, 1.0)), d, strobe_mixWeight(m, 3.0, 2.0)); }
vec2 _mix4(vec2 a, vec2 b, vec2 c, vec2 d, float m) { return mix(mix(mix(a, b, strobe_mixWeight(m, 3.0, 0.0)), c, strobe_mixWeight(m, 3.0, 1.0)), d, strobe_mixWeight(m, 3.0, 2.0)); }
vec3 _mix4(vec3 a, vec3 b, vec3 c, vec3 d, float m) { return mix(mix(mix(a, b, strobe_mixWeight(m, 3.0, 0.0)), c, strobe_mixWeight(m, 3.0, 1.0)), d, strobe_mixWeight(m, 3.0, 2.0)); }
vec4 _mix4(vec4 a, vec4 b, vec4 c, vec4 d, float m) { return mix(mix(mix(a, b, strobe_mixWeight(m, 3.0, 0.0)), c, strobe_mixWeight(m, 3.0, 1.0)), d, strobe_mixWeight(m, 3.0, 2.0)); }
float _mix5(float a, float b, float c, float d, float e, float m) { return mix(mix(mix(mix(a, b, strobe_mixWeight(m, 4.0, 0.0)), c, strobe_mixWeight(m, 4.0, 1.0)), d, strobe_mixWeight(m, 4.0, 2.0)), e, strobe_mixWeight(m, 4.0, 3.0)); }
vec2 _mix5(vec2 a, vec2 b, vec2 c, vec2 d, vec2 e, float m) { return mix(mix(mix(mix(a, b, strobe_mixWeight(m, 4.0, 0.0)), c, strobe_mixWeight(m, 4.0, 1.0)), d, strobe_mixWeight(m, 4.0, 2.0)), e, strobe_mixWeight(m, 4.0, 3.0)); }
vec3 _mix5(vec3 a, vec3 b, vec3 c, vec3 d, vec3 e, float m) { return mix(mix(mix(mix(a, b, strobe_mixWeight(m, 4.0, 0.0)), c, strobe_mixWeight(m, 4.0, 1.0)), d, strobe_mixWeight(m, 4.0, 2.0)), e, strobe_mixWeight(m, 4.0, 3.0)); }
vec4 _mix5(vec4 a, vec4 b, vec4 c, vec4 d, vec4 e, float m) { return mix(mix(mix(mix(a, b, strobe_mixWeight(m, 4.0, 0.0)), c, strobe_mixWeight(m, 4.0, 1.0)), d, strobe_mixWeight(m, 4.0, 2.0)), e, strobe_mixWeight(m, 4.0, 3.0)); }

float _nsin(float v) { return sin(v) * 0.5 + 0.5; }
float _ncos(float v) { return cos(v) * 0.5 + 0.5; }

float _nclamp(float v) { return clamp(v, 0.0, 1.0); }
vec2 _nclamp(vec2 v) { return clamp(v, 0.0, 1.0); }
vec3 _nclamp(vec3 v) { return clamp(v, 0.0, 1.0); }
vec4 _nclamp(vec4 v) { return clamp(v, 0.0, 1.0); }

// ---------------------------------------------------------------- hashes
// Sine-free hashes (after Dave Hoskins): stable across GPUs where
// fract(sin(x) * big) is not, and cheap. All return 0..1.

float _hash11(float p) { p = fract(p * 0.1031); p *= p + 33.33; p *= p + p; return fract(p); }
float _hash12(vec2 p) { vec3 p3 = fract(vec3(p.xyx) * 0.1031); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.x + p3.y) * p3.z); }
float _hash13(vec3 p3) { p3 = fract(p3 * 0.1031); p3 += dot(p3, p3.zyx + 31.32); return fract((p3.x + p3.y) * p3.z); }
vec2 _hash21(float p) { vec3 p3 = fract(vec3(p) * vec3(0.1031, 0.1030, 0.0973)); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.xx + p3.yz) * p3.zy); }
vec2 _hash22(vec2 p) { vec3 p3 = fract(vec3(p.xyx) * vec3(0.1031, 0.1030, 0.0973)); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.xx + p3.yz) * p3.zy); }
vec2 _hash23(vec3 p3) { p3 = fract(p3 * vec3(0.1031, 0.1030, 0.0973)); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.xx + p3.yz) * p3.zy); }
vec3 _hash31(float p) { vec3 p3 = fract(vec3(p) * vec3(0.1031, 0.1030, 0.0973)); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.xxy + p3.yzz) * p3.zyx); }
vec3 _hash32(vec2 p) { vec3 p3 = fract(vec3(p.xyx) * vec3(0.1031, 0.1030, 0.0973)); p3 += dot(p3, p3.yxz + 33.33); return fract((p3.xxy + p3.yzz) * p3.zyx); }
vec3 _hash33(vec3 p3) { p3 = fract(p3 * vec3(0.1031, 0.1030, 0.0973)); p3 += dot(p3, p3.yxz + 33.33); return fract((p3.xxy + p3.yxx) * p3.zyx); }
vec4 _hash41(float p) { vec4 p4 = fract(vec4(p) * vec4(0.1031, 0.1030, 0.0973, 0.1099)); p4 += dot(p4, p4.wzxy + 33.33); return fract((p4.xxyz + p4.yzzw) * p4.zywx); }
vec4 _hash42(vec2 p) { vec4 p4 = fract(vec4(p.xyxy) * vec4(0.1031, 0.1030, 0.0973, 0.1099)); p4 += dot(p4, p4.wzxy + 33.33); return fract((p4.xxyz + p4.yzzw) * p4.zywx); }
vec4 _hash43(vec3 p) { vec4 p4 = fract(vec4(p.xyzx) * vec4(0.1031, 0.1030, 0.0973, 0.1099)); p4 += dot(p4, p4.wzxy + 33.33); return fract((p4.xxyz + p4.yzzw) * p4.zywx); }
vec4 _hash44(vec4 p4) { p4 = fract(p4 * vec4(0.1031, 0.1030, 0.0973, 0.1099)); p4 += dot(p4, p4.wzxy + 33.33); return fract((p4.xxyz + p4.yzzw) * p4.zywx); }

// ---------------------------------------------------------------- noise
// Value noise, 0..1, as Synesthesia's engine computes it (D491), so the 21
// shipped scenes that call these see the same clouds: in 1D a straight line
// between `_rand` at the integers, in 2D a smoothstep between them, and in 3D
// a permutation hash (the 289 polynomial) with smoothstep interpolation.

float _noise(float x) {
    float i = floor(x);
    return mix(_rand(i), _rand(i + 1.0), fract(x));
}

float _noise(vec2 x) {
    vec2 i = floor(x);
    vec2 u = smoothstep(vec2(0.0), vec2(1.0), fract(x));
    return mix(mix(_rand(i), _rand(i + vec2(1.0, 0.0)), u.x),
               mix(_rand(i + vec2(0.0, 1.0)), _rand(i + vec2(1.0, 1.0)), u.x), u.y);
}

vec4 strobe_mod289(vec4 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec4 strobe_perm(vec4 x) { return strobe_mod289((x * 34.0 + 1.0) * x); }

float _noise(vec3 x) {
    vec3 i = floor(x);
    vec3 f = x - i;
    f = f * f * (3.0 - 2.0 * f);
    vec4 corner = i.xxyy + vec4(0.0, 1.0, 0.0, 1.0);
    vec4 row = strobe_perm(strobe_perm(corner.xyxy).xyxy + corner.zzww) + i.zzzz;
    vec4 near = fract(strobe_perm(row) * (1.0 / 41.0));
    vec4 far = fract(strobe_perm(row + 1.0) * (1.0 / 41.0));
    vec4 z = mix(near, far, f.z);
    vec2 xy = mix(z.xz, z.yw, f.x);
    return mix(xy.x, xy.y, f.y);
}

// Octaves of halving weight, not normalised (five sum to 0.97), each at twice
// the frequency and shifted by 100; in 2D also turned by half a radian (D491).
// Octaves are capped at 10; the loop bound must be constant in GLSL ES.
float _fbm(float x, float octaves) {
    float sum = 0.0; float amp = 0.5;
    for (int i = 0; i < 10; i++) {
        if (float(i) >= octaves) { break; }
        sum += amp * _noise(x); x = x * 2.0 + 100.0; amp *= 0.5;
    }
    return sum;
}
float _fbm(vec2 x, float octaves) {
    float sum = 0.0; float amp = 0.5;
    mat2 rot = mat2(cos(0.5), sin(0.5), -sin(0.5), cos(0.5));
    for (int i = 0; i < 10; i++) {
        if (float(i) >= octaves) { break; }
        sum += amp * _noise(x); x = rot * x * 2.0 + vec2(100.0); amp *= 0.5;
    }
    return sum;
}
float _fbm(vec3 x, float octaves) {
    float sum = 0.0; float amp = 0.5;
    for (int i = 0; i < 10; i++) {
        if (float(i) >= octaves) { break; }
        sum += amp * _noise(x); x = x * 2.0 + vec3(100.0); amp *= 0.5;
    }
    return sum;
}
float _fbm(float x) { return _fbm(x, 5.0); }
float _fbm(vec2 x) { return _fbm(x, 5.0); }
float _fbm(vec3 x) { return _fbm(x, 5.0); }

// Smooth, never repeating, stateless, -1..1: the sine of a weighted sum of
// sines at the first ten primes, the n-th weighted n (from 0), as
// Synesthesia's moves (D491).
// Written out (the 0th term, at 2, weighs nothing) so no local `const` joins
// the names the loader reserves (`names::library_names`).
float _statelessContinuousChaotic(float t) {
    return sin(sin(3.0 * t) + 2.0 * sin(5.0 * t) + 3.0 * sin(7.0 * t)
        + 4.0 * sin(11.0 * t) + 5.0 * sin(13.0 * t) + 6.0 * sin(17.0 * t)
        + 7.0 * sin(19.0 * t) + 8.0 * sin(23.0 * t) + 9.0 * sin(29.0 * t));
}

// ---------------------------------------------------------------- coordinates

// 0..1 to -1..1 and back, with no aspect correction: what Synesthesia's
// engine does (D491), although `_uvc` itself is aspect-corrected, so these
// are not `_uvc`'s inverse.
void _uv2uvc(inout vec2 p) { p = p * 2.0 - 1.0; }
void _uvc2uv(inout vec2 p) { p = p * 0.5 + 0.5; }

// Rotates about the origin by theta **radians, clockwise**. The SSF
// documentation says degrees, but Synesthesia's engine takes radians and
// turns this way, and real scenes call it with `PI / 4` and the like (D491).
vec2 _rotate(vec2 v, float theta) {
    float c = cos(theta); float s = sin(theta);
    return vec2(c * v.x + s * v.y, c * v.y - s * v.x);
}

float _mirror(float v) { return 1.0 - abs(1.0 - mod(v, 2.0)); }
vec2 _mirror(vec2 v) { return 1.0 - abs(1.0 - mod(v, 2.0)); }
vec3 _mirror(vec3 v) { return 1.0 - abs(1.0 - mod(v, 2.0)); }
vec4 _mirror(vec4 v) { return 1.0 - abs(1.0 - mod(v, 2.0)); }

// Without quadrant correction, as documented: theta covers -PI/2..PI/2.
vec2 _toPolar(vec2 xy) {
    float x = abs(xy.x) < 0.000001 ? 0.000001 : xy.x;
    return vec2(length(xy), atan(xy.y / x));
}
// With quadrant correction; theta is in turns, 0..1 counterclockwise from +x,
// as Synesthesia's engine returns it (D491). The origin gives 0.
vec2 _toPolarTrue(vec2 xy) {
    float turns = (xy.x == 0.0 && xy.y == 0.0) ? 0.0 : atan(xy.y, xy.x) / 6.28318530718;
    return vec2(length(xy), fract(turns + 1.0));
}
vec2 _toRect(vec2 rt) { return rt.x * vec2(cos(rt.y), sin(rt.y)); }

// ---------------------------------------------------------------- color

vec3 _rgb2hsv(vec3 c) {
    vec4 K = vec4(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
    vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
    float d = q.x - min(q.w, q.y);
    float e = 1.0e-10;
    return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

vec3 _hsv2rgb(vec3 c) {
    vec3 p = abs(fract(c.xxx + vec3(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0);
    return c.z * mix(vec3(1.0), clamp(p - 1.0, 0.0, 1.0), c.y);
}

vec3 _normalizeRGB(float r, float g, float b) { return vec3(r, g, b) / 255.0; }

vec3 _palette(float t, vec3 a, vec3 b, vec3 c, vec3 d) { return a + b * cos(6.28318530718 * (c * t + d)); }

vec3 _brightness(vec3 c, float b) { return c * b; }
vec4 _brightness(vec4 c, float b) { return vec4(c.rgb * b, c.a); }

vec3 _gamma(vec3 c, float g) { return pow(max(c, vec3(0.0)), vec3(1.0 / max(g, 0.0001))); }
vec4 _gamma(vec4 c, float g) { return vec4(_gamma(c.rgb, g), c.a); }

vec3 _contrast(vec3 c, float k) { return (c - 0.5) * k + 0.5; }
vec4 _contrast(vec4 c, float k) { return vec4(_contrast(c.rgb, k), c.a); }

float _luminance(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }
float _luminance(vec4 c) { return _luminance(c.rgb); }

// Scales the HSV saturation (D491): 0.5 half as saturated, 1 no change.
vec3 _saturation(vec3 c, float s) { vec3 hsv = _rgb2hsv(c); hsv.y *= s; return _hsv2rgb(hsv); }
vec4 _saturation(vec4 c, float s) { return vec4(_saturation(c.rgb, s), c.a); }

// Rotation about the grey axis, by degrees.
vec3 _hueRotate(vec3 c, float degrees) {
    float r = radians(degrees);
    float cs = cos(r); float sn = sin(r);
    vec3 k = vec3(0.57735);
    return c * cs + cross(k, c) * sn + k * dot(k, c) * (1.0 - cs);
}
vec4 _hueRotate(vec4 c, float degrees) { return vec4(_hueRotate(c.rgb, degrees), c.a); }

// Undocumented; Synesthesia's engine defines them (R17, D491). Both work in
// HSV on the colour clamped to 0..1, with hue in turns added to H, and
// neutral at (0, 0, _). The current form scales S by 1 + 2.5 * s * |s|; the
// LEGACY form adds `saturation` to S. Neither uses its `contrast` argument:
// Synesthesia applies the output's contrast meta control there, which Strobe
// applies once in its post stage, so within the scene it is neutral. The one
// real call, (0.0, 0.5, 1.0) in hue_review, raises S by a half.
vec4 strobe_hsvShift(vec4 c, float hue, float scale, float add) {
    vec3 hsv = _rgb2hsv(clamp(c.rgb, 0.0, 1.0));
    hsv.x += hue;
    hsv.y = clamp(hsv.y * scale + add, 0.0, 1.0);
    return vec4(_hsv2rgb(hsv), c.a);
}
vec4 _hueSaturationContrast(vec4 c, float hue, float saturation, float contrast) {
    return strobe_hsvShift(c, hue, 1.0 + 2.5 * saturation * abs(saturation), 0.0);
}
vec4 _hueSaturationContrastLEGACY(vec4 c, float hue, float saturation, float contrast) {
    return strobe_hsvShift(c, hue, 1.0, saturation);
}

vec3 _invert(vec3 c, float amount) { return mix(c, 1.0 - c, amount); }
vec4 _invert(vec4 c, float amount) { return vec4(_invert(c.rgb, amount), c.a); }

vec3 _grayscale(vec3 c, float intensity) { return mix(c, vec3(_luminance(c)), clamp(intensity, 0.0, 1.0)); }
vec4 _grayscale(vec4 c, float intensity) { return vec4(_grayscale(c.rgb, intensity), c.a); }
vec3 _grayscale(vec3 c) { return _grayscale(c, 1.0); }
vec4 _grayscale(vec4 c) { return _grayscale(c, 1.0); }

// A gradient black -> dark -> light -> white over the square root of the
// luminance, in three equal steps, as Synesthesia's (D491).
vec3 _dichrome(vec3 c, vec3 dark, vec3 light) {
    float t = clamp(sqrt(max(_luminance(c), 0.0)) * 3.0, 0.0, 3.0);
    if (t <= 1.0) { return mix(vec3(0.0), dark, t); }
    if (t <= 2.0) { return mix(dark, light, t - 1.0); }
    return mix(light, vec3(1.0), t - 2.0);
}
vec4 _dichrome(vec4 c, vec3 dark, vec3 light) { return vec4(_dichrome(c.rgb, dark, light), c.a); }
// The documentation spells it both ways.
vec3 _diChrome(vec3 c, vec3 dark, vec3 light) { return _dichrome(c, dark, light); }
vec4 _diChrome(vec4 c, vec3 dark, vec3 light) { return _dichrome(c, dark, light); }

// ---------------------------------------------------------------- media

bool _isMediaActive() { return syn_MediaType > 0.5; }

// "Cover" fit: a picture of `size` fills the frame without stretching.
vec2 _correctImageCoords(vec2 size, vec2 uv) {
    float frameAspect = RENDERSIZE.x / RENDERSIZE.y;
    float mediaAspect = size.x / max(size.y, 1.0);
    vec2 c = uv - 0.5;
    if (frameAspect > mediaAspect) {
        c.y *= mediaAspect / frameAspect;
    } else {
        c.x *= frameAspect / mediaAspect;
    }
    return c + 0.5;
}
// Real scenes pass `textureSize(tex, 0)` straight in.
vec2 _correctImageCoords(ivec2 size, vec2 uv) { return _correctImageCoords(vec2(size), uv); }
// The one-argument form real scenes call is for this pixel (TORNADO T5).
vec2 _correctImageCoords(vec2 size) { return _correctImageCoords(size, _uv); }
vec2 _correctImageCoords(ivec2 size) { return _correctImageCoords(vec2(size), _uv); }
// Given the frame-to-image size ratios per axis, the one to divide both by
// so the image covers the frame: Synesthesia's with its Fit or Fill media
// control on, its usual setting (R18).
float _ratioFix(float widthRatio, float heightRatio) { return max(widthRatio, heightRatio); }
vec2 _correctMediaCoords(vec2 uv) { return _correctImageCoords(vec2(textureSize(syn_Media, 0)), uv); }
vec2 _correctMediaCoords() { return _correctMediaCoords(_uv); }

vec4 _textureMedia(vec2 uv) { return texture(syn_Media, _correctMediaCoords(uv)); }
vec4 _textureMedia(vec2 uv, float lod) { return textureLod(syn_Media, _correctMediaCoords(uv), lod); }

// The offset is added after the aspect correction, in the picture's own 0..1
// space, and a coordinate that leaves the picture reads black, as
// Synesthesia's engine does (D491).
vec4 strobe_mediaAt(vec2 uv, float lod) { return _inRange(uv) < 0.5 ? vec4(0.0) : textureLod(syn_Media, uv, lod); }
vec4 strobe_mediaAt(vec2 uv) { return _inRange(uv) < 0.5 ? vec4(0.0) : texture(syn_Media, uv); }
vec4 _loadMedia() { return strobe_mediaAt(_correctMediaCoords(_uv)); }
vec4 _loadMedia(float lod) { return strobe_mediaAt(_correctMediaCoords(_uv), lod); }
vec4 _loadMedia(vec2 offset) { return strobe_mediaAt(_correctMediaCoords(_uv) + offset); }
vec4 _loadMedia(vec2 offset, float lod) { return strobe_mediaAt(_correctMediaCoords(_uv) + offset, lod); }

// White where the luminance reaches half the media contrast (0.5 at its
// neutral 1.0), alpha included (D491).
vec4 strobe_asMask(vec4 c) { return vec4(step(0.5, _luminance(c.rgb))); }
vec4 _loadMediaAsMask() { return strobe_asMask(_loadMedia()); }
vec4 _loadMediaAsMask(vec2 offset) { return strobe_asMask(_loadMedia(offset)); }
vec4 _textureMediaAsMask(vec2 uv) { return strobe_asMask(_textureMedia(uv)); }

// The same family for any image (D197). Synesthesia's editor lists these
// names; the docs describe only the media versions, so each mirrors its
// media counterpart with `img` in place of syn_Media.
vec4 _textureImage(sampler2D img, vec2 uv) { return texture(img, _correctImageCoords(vec2(textureSize(img, 0)), uv)); }
// As for the media, the offset is in the picture's space; only the
// unshifted coordinate is range-checked, as in Synesthesia's engine (D491).
vec4 _loadImage(sampler2D img, vec2 offset) {
    vec2 uv = _correctImageCoords(vec2(textureSize(img, 0)), _uv);
    return _inRange(uv) < 0.5 ? vec4(0.0) : texture(img, uv + offset);
}
vec4 _loadImage(sampler2D img) { return _loadImage(img, vec2(0.0)); }
vec4 _imageToMask(vec4 c) { return strobe_asMask(c); }
vec4 _loadImageAsMask(sampler2D img) { return strobe_asMask(_loadImage(img)); }
vec4 _loadImageAsMask(sampler2D img, vec2 offset) { return strobe_asMask(_loadImage(img, offset)); }
vec4 _textureImageAsMask(sampler2D img, vec2 uv) { return strobe_asMask(_textureImage(img, uv)); }

// Older names in real scenes (D197): the user image was the media input
// before it was called that, and video arrives upside down relative to
// stills.
#define syn_UserImage syn_Media
#define _loadUserImage _loadMedia
#define _loadUserImageAsMask _loadMediaAsMask
vec2 _invertYAxisVideo(vec2 uv) { return vec2(uv.x, 1.0 - uv.y); }
// Strobe has no per-media flip or invert option yet, so these are the
// identity they are when those options are off.
vec2 _flipMediaCoords(vec2 uv) { return uv; }
vec2 _applyMediaScale(vec2 uv) { return uv; }
vec4 _applyInvertMedia(vec4 c) { return c; }
vec4 _invertImage(vec4 c) { return c; }
vec4 _applyMediaContrast(vec4 c) { return c; }
// The media panel's contrast, which a scene can apply itself with
// `_contrast(c, _Media_Contrast)` (SkaiWalkerOG, D268): neutral, as above.
const float _Media_Contrast = 1.0;
// Whether a sampler has a picture: an unbound one is Strobe's 1x1 black.
// A 1-pixel-wide strip still counts (D491).
bool _exists(sampler2D tex) { ivec2 size = textureSize(tex, 0); return size.x > 1 || size.y > 1; }

// Audio names real scenes use that neither Synesthesia's docs nor its
// editor list (D197). These are stand-ins until their meaning is known:
// the input level as the overall level, and an exponential bass curve as
// the bass level squared.
#define syn_InputLevel syn_Level
#define syn_BassExpoCurve (syn_BassLevel * syn_BassLevel)

// ---------------------------------------------------------------- edges

// The cheap edge: how much the texture changes from this pixel to its
// neighbours, per channel, from the screen-space derivatives of a read half a
// pixel (of the output) along, as Synesthesia's engine does (D491).
vec4 _edgeDetect(sampler2D tex, vec2 uv) {
    vec4 c = texture(tex, uv + 0.5 / RENDERSIZE);
    return max(abs(dFdx(c)), abs(dFdy(c)));
}
vec4 _edgeDetect(sampler2D tex) { return _edgeDetect(tex, _uv); }

// Sobel over each tap's RGB length, one output pixel apart, in every channel
// alpha included (D491).
float strobe_tap(sampler2D tex, vec2 uv) { return length(texture(tex, uv).rgb); }
vec4 _edgeDetectSobel(sampler2D tex, vec2 uv) {
    vec2 px = 1.0 / RENDERSIZE;
    float tl = strobe_tap(tex, uv + vec2(-1.0, 1.0) * px);
    float t  = strobe_tap(tex, uv + vec2(0.0, 1.0) * px);
    float tr = strobe_tap(tex, uv + vec2(1.0, 1.0) * px);
    float l  = strobe_tap(tex, uv + vec2(-1.0, 0.0) * px);
    float r  = strobe_tap(tex, uv + vec2(1.0, 0.0) * px);
    float bl = strobe_tap(tex, uv + vec2(-1.0, -1.0) * px);
    float b  = strobe_tap(tex, uv + vec2(0.0, -1.0) * px);
    float br = strobe_tap(tex, uv + vec2(1.0, -1.0) * px);
    float gx = -tl - 2.0 * l - bl + tr + 2.0 * r + br;
    float gy = -bl - 2.0 * b - br + tl + 2.0 * t + tr;
    return vec4(sqrt(gx * gx + gy * gy));
}
vec4 _edgeDetectSobel(sampler2D tex) { return _edgeDetectSobel(tex, _uv); }

vec4 _edgeDetectMedia() {
    vec2 uv = _correctMediaCoords(_uv);
    return _inRange(uv) < 0.5 ? vec4(0.0) : _edgeDetect(syn_Media, uv);
}
vec4 _edgeDetectSobelMedia() {
    vec2 uv = _correctMediaCoords(_uv);
    return _inRange(uv) < 0.5 ? vec4(0.0) : _edgeDetectSobel(syn_Media, uv);
}
// A pass's edges, by the name real scenes use (D197). Synesthesia documents
// it as returning a float, and real scenes assign it to one (TORNADO T5).
float syn_pass_edgeDetectSobel(sampler2D pass) { return _edgeDetectSobel(pass).r; }

// ---------------------------------------------------------------- tilt shift
// A separable Gaussian whose radius grows with distance from blurLoc along
// the other axis. blurAmt sets the tap count (capped at 24 each side).

vec4 strobe_tiltShift(sampler2D tex, vec2 uv, float blurAmt, float blurLoc, vec2 axis, float across) {
    vec2 px = 1.0 / vec2(textureSize(tex, 0));
    float radius = abs(across - blurLoc) * 2.0 * blurAmt;
    float taps = clamp(radius, 0.0, 24.0);
    vec4 sum = texture(tex, uv);
    float weight = 1.0;
    for (int i = 1; i <= 24; i++) {
        if (float(i) > taps) { break; }
        float w = exp(-float(i * i) / max(2.0 * taps * taps / 9.0, 0.0001));
        sum += w * (texture(tex, uv + axis * px * float(i)) + texture(tex, uv - axis * px * float(i)));
        weight += 2.0 * w;
    }
    return sum / weight;
}

vec4 _hBlurTiltShift(sampler2D tex, vec2 uv, float blurAmt, float blurLoc) { return strobe_tiltShift(tex, uv, blurAmt, blurLoc, vec2(1.0, 0.0), uv.y); }
vec4 _hBlurTiltShift(sampler2D tex, vec2 uv, float blurAmt) { return _hBlurTiltShift(tex, uv, blurAmt, 0.5); }
vec4 _hBlurTiltShift(sampler2D tex, vec2 uv) { return _hBlurTiltShift(tex, uv, 5.0, 0.5); }
vec4 _hBlurTiltShift(sampler2D tex) { return _hBlurTiltShift(tex, _uv, 5.0, 0.5); }
// The documentation's example calls the uv-less form.
vec4 _hBlurTiltShift(sampler2D tex, float blurAmt, float blurLoc) { return _hBlurTiltShift(tex, _uv, blurAmt, blurLoc); }

vec4 _vBlurTiltShift(sampler2D tex, vec2 uv, float blurAmt, float blurLoc) { return strobe_tiltShift(tex, uv, blurAmt, blurLoc, vec2(0.0, 1.0), uv.y); }
vec4 _vBlurTiltShift(sampler2D tex, vec2 uv, float blurAmt) { return _vBlurTiltShift(tex, uv, blurAmt, 0.5); }
vec4 _vBlurTiltShift(sampler2D tex, vec2 uv) { return _vBlurTiltShift(tex, uv, 5.0, 0.5); }
vec4 _vBlurTiltShift(sampler2D tex) { return _vBlurTiltShift(tex, _uv, 5.0, 0.5); }
vec4 _vBlurTiltShift(sampler2D tex, float blurAmt, float blurLoc) { return _vBlurTiltShift(tex, _uv, blurAmt, blurLoc); }
