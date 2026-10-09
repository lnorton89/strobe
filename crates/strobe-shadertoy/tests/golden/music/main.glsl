// ---- Strobe import of Shadertoy Tst007 "Music input" by strobe-tests (generated) ----
// Pass: Image. Shim above the first marker; Shadertoy code below, unedited.
// Tab line 1 is file line 22 for Image.
vec2 strobe_fragCoord = vec2(0.0);
vec3 strobe_iResolution;
vec3 strobe_iChannelResolution[4] = vec3[4](vec3(0.0), vec3(0.0), vec3(0.0), vec3(0.0));
void strobe_begin() {
    strobe_iResolution = vec3(RENDERSIZE, 1.0);
    strobe_iChannelResolution[0] = vec3(vec2(textureSize(strobe_ShadertoyAudio, 0)), 1.0);
    strobe_fragCoord = (gl_FragCoord.xy - 0.5 * RENDERSIZE) / st_zoom + 0.5 * RENDERSIZE;
}
vec4 strobe_end(vec4 c) {
    c.rgb *= 1.0 + st_bass_flash * syn_BassHits;
    if (st_hue != 0.0) { c.rgb = _hueRotate(c.rgb, st_hue); }
    c.a = 1.0;
    return c;
}
#define iResolution strobe_iResolution
#define iChannelResolution strobe_iChannelResolution
#define iChannel0 strobe_ShadertoyAudio
// ---- Shadertoy tab: Image ----
void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    vec2 uv = fragCoord / iResolution.xy;
    float fft = texture(iChannel0, vec2(uv.x, 0.25)).x;
    float wave = texture(iChannel0, vec2(uv.x, 0.75)).x;
    fragColor = vec4(fft, wave, 0.1 + 0.2 * iChannelResolution[0].x / 512.0, 1.0);
}
// ---- Strobe entry (generated) ----
vec4 renderMain() {
    vec4 strobe_color = vec4(0.0, 0.0, 0.0, 1.0);
    strobe_begin();
    mainImage(strobe_color, strobe_fragCoord);
    return strobe_end(strobe_color);
}
