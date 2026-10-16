// Voronoi Glass — Strobe base library, tier 2.
// Two-pass voronoi (nearest point, then distance to the border) over a 3x3
// and 5x5 neighbourhood: the classic exact-border method.

// Where the feature point of cell c sits, in cell units.
vec2 cellPoint(vec2 c, float t) {
    return 0.5 + 0.5 * sin(t + 6.2831 * _hash22(c));
}

// Distance from the pixel (at f inside its own cell) to the box of the cell
// at offset g: a lower bound on the distance to that cell's feature point.
float boxDist2(vec2 g, vec2 f) {
    vec2 d = max(max(g - f, f - g - 1.0), 0.0);
    return dot(d, d);
}

// Both searches reject a cell by distance before hashing it. The rejection
// is exact, not an approximation: the nearest-point search skips a cell whose
// box is already farther than the best point so far, and the border search
// skips a cell that cannot bring the border distance below `reach`, the
// largest value that still changes the picture (the lead is fully open from
// edge + 0.02 up). Border distance to a point at distance |r| from the pixel
// is at least (|r| - |mr|) / 2, with mr the nearest point.
vec3 voronoi(vec2 x, float t, float reach) {
    vec2 n = floor(x);
    vec2 f = fract(x);
    vec2 mg = vec2(0.0);
    vec2 mr = cellPoint(n, t) - f;
    float md = dot(mr, mr);
    for (int j = -1; j <= 1; j++) {
        for (int i = -1; i <= 1; i++) {
            if (i == 0 && j == 0) { continue; }
            vec2 g = vec2(float(i), float(j));
            if (boxDist2(g, f) >= md) { continue; }
            vec2 r = g + cellPoint(n + g, t) - f;
            float d = dot(r, r);
            if (d < md) { md = d; mr = r; mg = g; }
        }
    }
    float lim = sqrt(md) + 2.0 * reach + 0.001;
    float lim2 = lim * lim;
    float mr2 = md;
    md = 8.0;
    for (int j = -2; j <= 2; j++) {
        for (int i = -2; i <= 2; i++) {
            vec2 g = mg + vec2(float(i), float(j));
            if (boxDist2(g, f) > lim2) { continue; }
            vec2 r = g + cellPoint(n + g, t) - f;
            vec2 dr = r - mr;
            float dd = dot(dr, dr);
            if (dd > 0.00001) {
                // dot(0.5 * (mr + r), normalize(r - mr)), without the normalize.
                md = min(md, 0.5 * (dot(r, r) - mr2) * inversesqrt(dd));
            }
        }
    }
    return vec3(md, n + mg);
}

vec4 renderMain(void) {
    vec2 p = _uvc * density;
    vec3 v = voronoi(p, TIME * 0.6 + syn_MidTime * 0.5, edge + 0.02);
    float id = _hash12(v.yz + floor(syn_BeatTime));
    vec3 glass = _hsv2rgb(vec3(id, 0.7, 0.9));
    glass *= 0.6 + 0.6 * syn_MidLevel + 0.5 * _hash12(v.yz) * syn_MidHits;

    float lead = smoothstep(edge, edge + 0.02, v.x);
    vec3 col = mix(lead_color, glass, lead);
    col += (1.0 - lead) * syn_HighHits * vec3(0.8, 0.9, 1.0);
    return vec4(col, 1.0);
}
