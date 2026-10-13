// ============================================================================
// SSF function library (Strobe implementation).
//
// Compiles as desktop GLSL 3.30 core and as GLSL ES 3.00. Every float literal
// carries a decimal point, no built-in is overloaded, and every loop bound is
// constant. Private helpers are prefixed strobe_. Depends on uniforms and
// globals the assembler declares first: RENDERSIZE, _xy, _uv, syn_Media,
// syn_MediaType.
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

float _rand(float seed) { return fract(sin(seed * 12.9898) * 43758.5453); }
float _rand(vec2 seed) { return fract(sin(dot(seed, vec2(12.9898, 78.233))) * 43758.5453); }

float _pulse(float position, float center, float size) {
    return 1.0 - smoothstep(0.0, max(size * 0.5, 0.00001), abs(position - center));
}
float _sqPulse(float position, float center, float size) {
    return step(abs(position - center), size * 0.5);
}

float _inRange(float v, float lo, float hi) { return step(lo, v) * step(v, hi); }
float _inRange(float v) { return _inRange(v, 0.0, 1.0); }
float _inRange(vec2 v, float lo, float hi) { return _inRange(v.x, lo, hi) * _inRange(v.y, lo, hi); }
float _inRange(vec2 v) { return _inRange(v, 0.0, 1.0); }
float _inRange(vec3 v, float lo, float hi) { return _inRange(v.xy, lo, hi) * _inRange(v.z, lo, hi); }
float _inRange(vec3 v) { return _inRange(v, 0.0, 1.0); }
float _inRange(vec4 v, float lo, float hi) { return _inRange(v.xyz, lo, hi) * _inRange(v.w, lo, hi); }
float _inRange(vec4 v) { return _inRange(v, 0.0, 1.0); }

// Ranges from -0.5 to 0.0, as documented.
float _triWave(float position, float period) {
    return abs(fract(position / period) - 0.5) - 0.5;
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
// Value noise with quintic interpolation, 0..1.

float _noise(float x) {
    float i = floor(x);
    float f = fract(x);
    float u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    return mix(_hash11(i), _hash11(i + 1.0), u);
}

float _noise(vec2 x) {
    vec2 i = floor(x);
    vec2 f = fract(x);
    vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    float a = _hash12(i);
    float b = _hash12(i + vec2(1.0, 0.0));
    float c = _hash12(i + vec2(0.0, 1.0));
    float d = _hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float _noise(vec3 x) {
    vec3 i = floor(x);
    vec3 f = fract(x);
    vec3 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    float n000 = _hash13(i);
    float n100 = _hash13(i + vec3(1.0, 0.0, 0.0));
    float n010 = _hash13(i + vec3(0.0, 1.0, 0.0));
    float n110 = _hash13(i + vec3(1.0, 1.0, 0.0));
    float n001 = _hash13(i + vec3(0.0, 0.0, 1.0));
    float n101 = _hash13(i + vec3(1.0, 0.0, 1.0));
    float n011 = _hash13(i + vec3(0.0, 1.0, 1.0));
    float n111 = _hash13(i + vec3(1.0, 1.0, 1.0));
    return mix(
        mix(mix(n000, n100, u.x), mix(n010, n110, u.x), u.y),
        mix(mix(n001, n101, u.x), mix(n011, n111, u.x), u.y),
        u.z);
}

// Octaves are capped at 10; the loop bound must be constant in GLSL ES.
float _fbm(float x, float octaves) {
    float sum = 0.0; float amp = 0.5; float norm = 0.0;
    for (int i = 0; i < 10; i++) {
        if (float(i) >= octaves) { break; }
        sum += amp * _noise(x); norm += amp; x = x * 2.03 + 17.0; amp *= 0.5;
    }
    return sum / max(norm, 0.0001);
}
float _fbm(vec2 x, float octaves) {
    float sum = 0.0; float amp = 0.5; float norm = 0.0;
    mat2 rot = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < 10; i++) {
        if (float(i) >= octaves) { break; }
        sum += amp * _noise(x); norm += amp; x = rot * x * 2.03 + vec2(17.0, 11.0); amp *= 0.5;
    }
    return sum / max(norm, 0.0001);
}
float _fbm(vec3 x, float octaves) {
    float sum = 0.0; float amp = 0.5; float norm = 0.0;
    for (int i = 0; i < 10; i++) {
        if (float(i) >= octaves) { break; }
        sum += amp * _noise(x); norm += amp; x = x * 2.03 + vec3(17.0, 11.0, 5.0); amp *= 0.5;
    }
    return sum / max(norm, 0.0001);
}
float _fbm(float x) { return _fbm(x, 5.0); }
float _fbm(vec2 x) { return _fbm(x, 5.0); }
float _fbm(vec3 x) { return _fbm(x, 5.0); }

// Incommensurate sines: smooth, never repeating, stateless. Roughly -1..1.
float _statelessContinuousChaotic(float t) {
    return (sin(t) + sin(t * 1.618034) + sin(t * 2.718282 + 1.3) + sin(t * 0.577216 + 2.1)) * 0.25;
}

// ---------------------------------------------------------------- coordinates

void _uv2uvc(inout vec2 p) { p = (p - 0.5) * vec2(RENDERSIZE.x / RENDERSIZE.y, 1.0); }
void _uvc2uv(inout vec2 p) { p = p / vec2(RENDERSIZE.x / RENDERSIZE.y, 1.0) + 0.5; }

// Rotates about the origin by theta degrees.
vec2 _rotate(vec2 v, float theta) {
    float r = radians(theta);
    float c = cos(r); float s = sin(r);
    return vec2(c * v.x - s * v.y, s * v.x + c * v.y);
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
vec2 _toPolarTrue(vec2 xy) { return vec2(length(xy), atan(xy.y, xy.x)); }
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

vec3 _saturation(vec3 c, float s) { return mix(vec3(_luminance(c)), c, s); }
vec4 _saturation(vec4 c, float s) { return vec4(_saturation(c.rgb, s), c.a); }

// Rotation about the grey axis, by degrees.
vec3 _hueRotate(vec3 c, float degrees) {
    float r = radians(degrees);
    float cs = cos(r); float sn = sin(r);
    vec3 k = vec3(0.57735);
    return c * cs + cross(k, c) * sn + k * dot(k, c) * (1.0 - cs);
}
vec4 _hueRotate(vec4 c, float degrees) { return vec4(_hueRotate(c.rgb, degrees), c.a); }

vec3 _invert(vec3 c, float amount) { return mix(c, 1.0 - c, amount); }
vec4 _invert(vec4 c, float amount) { return vec4(_invert(c.rgb, amount), c.a); }

vec3 _grayscale(vec3 c, float intensity) { return mix(c, vec3(_luminance(c)), intensity); }
vec4 _grayscale(vec4 c, float intensity) { return vec4(_grayscale(c.rgb, intensity), c.a); }
vec3 _grayscale(vec3 c) { return _grayscale(c, 1.0); }
vec4 _grayscale(vec4 c) { return _grayscale(c, 1.0); }

vec3 _dichrome(vec3 c, vec3 dark, vec3 light) { return mix(dark, light, clamp(_luminance(c), 0.0, 1.0)); }
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
// so the image covers the frame, as `_correctImageCoords` does (Q27).
float _ratioFix(float widthRatio, float heightRatio) { return max(widthRatio, heightRatio); }
vec2 _correctMediaCoords(vec2 uv) { return _correctImageCoords(vec2(textureSize(syn_Media, 0)), uv); }
vec2 _correctMediaCoords() { return _correctMediaCoords(_uv); }

vec4 _textureMedia(vec2 uv) { return texture(syn_Media, _correctMediaCoords(uv)); }
vec4 _textureMedia(vec2 uv, float lod) { return textureLod(syn_Media, _correctMediaCoords(uv), lod); }

vec4 _loadMedia() { return _textureMedia(_uv); }
vec4 _loadMedia(float lod) { return _textureMedia(_uv, lod); }
vec4 _loadMedia(vec2 offset) { return _textureMedia(_uv + offset); }
vec4 _loadMedia(vec2 offset, float lod) { return _textureMedia(_uv + offset, lod); }

vec4 strobe_asMask(vec4 c) { return vec4(vec3(step(0.5, _luminance(c.rgb))), 1.0); }
vec4 _loadMediaAsMask() { return strobe_asMask(_loadMedia()); }
vec4 _loadMediaAsMask(vec2 offset) { return strobe_asMask(_loadMedia(offset)); }
vec4 _textureMediaAsMask(vec2 uv) { return strobe_asMask(_textureMedia(uv)); }

// The same family for any image (D197). Synesthesia's editor lists these
// names; the docs describe only the media versions, so each mirrors its
// media counterpart with `img` in place of syn_Media.
vec4 _textureImage(sampler2D img, vec2 uv) { return texture(img, _correctImageCoords(vec2(textureSize(img, 0)), uv)); }
vec4 _loadImage(sampler2D img) { return _textureImage(img, _uv); }
vec4 _loadImage(sampler2D img, vec2 offset) { return _textureImage(img, _uv + offset); }
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
// Whether a sampler has a picture: an unbound one is Strobe's 1x1 black.
bool _exists(sampler2D tex) { return textureSize(tex, 0).x > 1; }

// Audio names real scenes use that neither Synesthesia's docs nor its
// editor list (D197). These are stand-ins until their meaning is known:
// the input level as the overall level, and an exponential bass curve as
// the bass level squared.
#define syn_InputLevel syn_Level
#define syn_BassExpoCurve (syn_BassLevel * syn_BassLevel)

// ---------------------------------------------------------------- edges

vec4 _edgeDetect(sampler2D tex, vec2 uv) {
    vec2 px = 1.0 / vec2(textureSize(tex, 0));
    vec4 a = texture(tex, uv + vec2(-0.5, -0.5) * px);
    vec4 b = texture(tex, uv + vec2(0.5, 0.5) * px);
    vec4 c = texture(tex, uv + vec2(-0.5, 0.5) * px);
    vec4 d = texture(tex, uv + vec2(0.5, -0.5) * px);
    return vec4(vec3(length(a.rgb - b.rgb) + length(c.rgb - d.rgb)), 1.0);
}
vec4 _edgeDetect(sampler2D tex) { return _edgeDetect(tex, _uv); }

vec4 _edgeDetectSobel(sampler2D tex, vec2 uv) {
    vec2 px = 1.0 / vec2(textureSize(tex, 0));
    float tl = _luminance(texture(tex, uv + vec2(-1.0, 1.0) * px).rgb);
    float t  = _luminance(texture(tex, uv + vec2(0.0, 1.0) * px).rgb);
    float tr = _luminance(texture(tex, uv + vec2(1.0, 1.0) * px).rgb);
    float l  = _luminance(texture(tex, uv + vec2(-1.0, 0.0) * px).rgb);
    float r  = _luminance(texture(tex, uv + vec2(1.0, 0.0) * px).rgb);
    float bl = _luminance(texture(tex, uv + vec2(-1.0, -1.0) * px).rgb);
    float b  = _luminance(texture(tex, uv + vec2(0.0, -1.0) * px).rgb);
    float br = _luminance(texture(tex, uv + vec2(1.0, -1.0) * px).rgb);
    float gx = -tl - 2.0 * l - bl + tr + 2.0 * r + br;
    float gy = -bl - 2.0 * b - br + tl + 2.0 * t + tr;
    return vec4(vec3(sqrt(gx * gx + gy * gy)), 1.0);
}
vec4 _edgeDetectSobel(sampler2D tex) { return _edgeDetectSobel(tex, _uv); }

vec4 _edgeDetectMedia() { return _isMediaActive() ? _edgeDetect(syn_Media, _correctMediaCoords(_uv)) : vec4(0.0); }
vec4 _edgeDetectSobelMedia() { return _isMediaActive() ? _edgeDetectSobel(syn_Media, _correctMediaCoords(_uv)) : vec4(0.0); }
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
