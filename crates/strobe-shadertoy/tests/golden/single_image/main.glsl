// ---- Strobe import of Shadertoy Tst001 "Single image" by strobe-tests (generated) ----
// Pass: Image. Shim above the first marker; Shadertoy code below, unedited.
// Tab line 1 is file line 22 for Image.
vec2 strobe_fragCoord = vec2(0.0);
vec4 strobe_clock = texelFetch(st_clock, ivec2(0, 0), 0);
vec3 strobe_iResolution = vec3(RENDERSIZE, 1.0);
float strobe_iTime = strobe_clock.x;
vec4 strobe_iMouse = _mouse;
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
#define iMouse strobe_iMouse
// ---- Shadertoy tab: Image ----
void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    vec2 uv = fragCoord / iResolution.xy;
    vec2 m = iMouse.xy / iResolution.xy;
    vec3 col = 0.5 + 0.5 * cos(iTime + uv.xyx + vec3(0.0, 2.0, 4.0));
    col += 0.3 * smoothstep(0.2, 0.0, distance(uv, m));
    fragColor = vec4(col, 0.25);
}
// ---- Strobe entry (generated) ----
vec4 renderMain() {
    vec4 strobe_color = vec4(0.0, 0.0, 0.0, 1.0);
    strobe_begin();
    mainImage(strobe_color, strobe_fragCoord);
    return strobe_end(strobe_color);
}
