// ---- Strobe import of Shadertoy Tst004 "Four buffer chain" by strobe-tests (generated) ----
// Pass: Buffer C. Shim above the first marker; Shadertoy code below, unedited.
// Tab line 1 is file line 11 for Buffer C.
vec2 strobe_fragCoord = vec2(0.0);
void strobe_begin() {
    strobe_fragCoord = gl_FragCoord.xy;
}
vec4 strobe_end(vec4 c) { return c; }
#define iChannel0 st_buf_d
// ---- Shadertoy tab: Buffer C ----
void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    fragColor = vec4(texelFetch(iChannel0, ivec2(fragCoord), 0).x, 0.0, 0.0, 1.0);
}
// ---- Strobe entry (generated) ----
vec4 renderMain() {
    vec4 strobe_color = vec4(0.0, 0.0, 0.0, 1.0);
    strobe_begin();
    mainImage(strobe_color, strobe_fragCoord);
    return strobe_end(strobe_color);
}
