// ---- Strobe import of Shadertoy Tst002 "Common and image" by strobe-tests (generated) ----
// Pass: Image. Shim above the first marker; Shadertoy code below, unedited.
// Tab line 1 is file line 27 for Image, 21 for Common.
vec2 strobe_fragCoord = vec2(0.0);
vec4 strobe_clock = texelFetch(st_clock, ivec2(0, 0), 0);
vec3 strobe_iResolution = vec3(RENDERSIZE, 1.0);
float strobe_iTime = strobe_clock.x;
void strobe_begin() {
    strobe_fragCoord = (gl_FragCoord.xy - 0.5 * RENDERSIZE) / st_zoom + 0.5 * RENDERSIZE;
}
vec4 strobe_end(vec4 c) {
    c.rgb *= 1.0 + st_bass_flash * syn_BassHits;
    if (st_hue != 0.0) { c.rgb = _hueRotate(c.rgb, st_hue); }
    c.a = 1.0;
    return c;
}
#define iResolution strobe_iResolution
#define iTime strobe_iTime
#define PI strobe_user_PI
// ---- Shadertoy tab: Common ----
const float PI = 3.14159265;
float wave(vec2 p, float t)
{
    return 0.5 + 0.5 * sin(6.0 * p.x + t) * cos(6.0 * p.y - t);
}
// ---- Shadertoy tab: Image ----
void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    vec2 uv = fragCoord / iResolution.xy;
    float w = wave(uv - 0.5, iTime);
    fragColor = vec4(vec3(w, 0.5 + 0.5 * sin(PI * uv.x), 0.4), 1.0);
}
// ---- Strobe entry (generated) ----
vec4 renderMain() {
    vec4 strobe_color = vec4(0.0, 0.0, 0.0, 1.0);
    strobe_begin();
    mainImage(strobe_color, strobe_fragCoord);
    return strobe_end(strobe_color);
}
