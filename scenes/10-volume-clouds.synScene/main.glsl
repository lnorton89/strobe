// Volume Clouds — Strobe base library, tier 5.

float density(vec3 p) {
    vec3 q = p + vec3(TIME * 0.3 * speed, 0.0, TIME * 0.6 * speed);
    float n = _fbm(q * 0.45, 4.0);
    // A deck of cloud between y = 0 and y = 1.6, puffier toward the middle.
    float slab = smoothstep(0.0, 0.5, p.y) * smoothstep(1.6, 0.9, p.y);
    float threshold = mix(0.68, 0.38, coverage);
    return clamp((n * slab - threshold) * 5.0, 0.0, 1.0);
}

vec4 renderMain(void) {
    vec3 ro = vec3(0.0, 2.3 + 0.15 * syn_BassLevel, 0.0);
    vec3 rd = normalize(vec3(_uvc.x, _uvc.y - 0.22, 1.0));
    vec3 sunDir = normalize(vec3(sun.x, sun.y, 1.0));

    float sunAmt = max(dot(rd, sunDir), 0.0);
    vec3 sky = mix(vec3(0.85, 0.45, 0.3), vec3(0.15, 0.2, 0.45), clamp(rd.y * 2.5 + 0.3, 0.0, 1.0));
    sky += vec3(1.0, 0.7, 0.4) * pow(sunAmt, 64.0) * (1.0 + syn_Level * 2.0);

    vec4 acc = vec4(0.0);
    float t = 0.2;
    float stepLen = 18.0 / steps;
    for (int i = 0; i < 80; i++) {
        if (float(i) >= steps || acc.a > 0.97) { break; }
        vec3 p = ro + rd * t;
        float d = density(p);
        if (d > 0.01) {
            // Light: march a few steps toward the sun.
            float shade = 0.0;
            for (int j = 1; j <= 5; j++) {
                shade += density(p + sunDir * float(j) * 0.35);
            }
            float light = exp(-shade * 0.9);
            vec3 c = mix(vec3(0.25, 0.22, 0.32), vec3(1.0, 0.75, 0.55) * 1.4, light);
            c += vec3(1.0, 0.6, 0.35) * pow(sunAmt, 6.0) * light;
            float a = d * 0.5;
            acc.rgb += (1.0 - acc.a) * a * c;
            acc.a += (1.0 - acc.a) * a;
        }
        t += stepLen * (0.8 + 0.4 * _hash12(_xy + float(i)));
    }
    vec3 col = sky * (1.0 - acc.a) + acc.rgb;
    return vec4(_gamma(col, 1.1), 1.0);
}
