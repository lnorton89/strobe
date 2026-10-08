// Voronoi Glass — Strobe base library, tier 2.
// Two-pass voronoi (nearest point, then distance to the border) over a 3x3
// and 5x5 neighbourhood: the classic exact-border method.

vec3 voronoi(vec2 x, float t) {
    vec2 n = floor(x);
    vec2 f = fract(x);
    vec2 mg = vec2(0.0);
    vec2 mr = vec2(0.0);
    float md = 8.0;
    for (int j = -1; j <= 1; j++) {
        for (int i = -1; i <= 1; i++) {
            vec2 g = vec2(float(i), float(j));
            vec2 o = _hash22(n + g);
            o = 0.5 + 0.5 * sin(t + 6.2831 * o);
            vec2 r = g + o - f;
            float d = dot(r, r);
            if (d < md) { md = d; mr = r; mg = g; }
        }
    }
    md = 8.0;
    for (int j = -2; j <= 2; j++) {
        for (int i = -2; i <= 2; i++) {
            vec2 g = mg + vec2(float(i), float(j));
            vec2 o = _hash22(n + g);
            o = 0.5 + 0.5 * sin(t + 6.2831 * o);
            vec2 r = g + o - f;
            if (dot(mr - r, mr - r) > 0.00001) {
                md = min(md, dot(0.5 * (mr + r), normalize(r - mr)));
            }
        }
    }
    return vec3(md, n + mg);
}

vec4 renderMain(void) {
    vec2 p = _uvc * density;
    vec3 v = voronoi(p, TIME * 0.6 + syn_MidTime * 0.5);
    float id = _hash12(v.yz + floor(syn_BeatTime));
    vec3 glass = _hsv2rgb(vec3(id, 0.7, 0.9));
    glass *= 0.6 + 0.6 * syn_MidLevel + 0.5 * _hash12(v.yz) * syn_MidHits;

    float lead = smoothstep(edge, edge + 0.02, v.x);
    vec3 col = mix(lead_color, glass, lead);
    col += (1.0 - lead) * syn_HighHits * vec3(0.8, 0.9, 1.0);
    return vec4(col, 1.0);
}
