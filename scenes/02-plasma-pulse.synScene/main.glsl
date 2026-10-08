// Plasma Pulse — Strobe base library, tier 1.

vec4 renderMain(void) {
    vec2 p = _uvc * (scale + syn_MidLevel * 2.0);
    float t = TIME * 0.7 + syn_MidTime * 0.3;

    float v = sin(p.x + t);
    v += sin((p.y + t) * 0.5);
    v += sin((p.x + p.y + t) * 0.5);
    vec2 c = p + vec2(sin(t / 3.0), cos(t / 2.0)) * 2.5;
    v += sin(sqrt(dot(c, c) + 1.0) + t);
    v *= 0.5;

    vec3 col = vec3(sin(v * PI), sin(v * PI + 2.094), sin(v * PI + 4.188)) * 0.5 + 0.5;
    col = mix(col, col * tint * 1.6, 0.6);
    col = _invert(col, invert);
    col += (syn_Hits * 0.35 + flash) * vec3(1.0);
    return vec4(col, 1.0);
}
