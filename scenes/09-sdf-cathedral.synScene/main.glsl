// SDF Cathedral — Strobe base library, tier 4.

float sdBox(vec3 p, vec3 b) {
    vec3 q = abs(p) - b;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
}

float map(vec3 p) {
    // Repeat every 4 units along z; mirror across x for the two aisles.
    vec3 q = p;
    q.z = mod(q.z, 4.0) - 2.0;
    q.x = abs(q.x);

    float floorD = p.y + 1.0;
    float pillar = length(q.xz - vec2(2.2, 0.0)) - 0.32 - 0.03 * sin(p.y * 12.0);
    // A pointed arch: the space under two offset circles, cut from a slab.
    vec2 a = vec2(q.x, p.y - 1.8);
    float arch = max(length(a - vec2(-0.9, 0.0)), length(a - vec2(0.9, 0.0))) - 3.0;
    float wall = sdBox(q - vec3(2.2, 2.4, 0.0), vec3(0.25, 1.8, 2.1));
    float nave = max(wall, arch);
    float rib = sdBox(vec3(q.x, p.y - 3.9, q.z), vec3(2.4, 0.08, 0.12));
    return min(min(floorD, pillar), min(nave, rib));
}

vec3 normalAt(vec3 p) {
    const vec2 k = vec2(1.0, -1.0);
    const float h = 0.002;
    return normalize(k.xyy * map(p + k.xyy * h) + k.yyx * map(p + k.yyx * h)
                   + k.yxy * map(p + k.yxy * h) + k.xxx * map(p + k.xxx * h));
}

float softShadow(vec3 ro, vec3 rd) {
    float res = 1.0;
    float t = 0.05;
    for (int i = 0; i < 28; i++) {
        float h = map(ro + rd * t);
        res = min(res, 10.0 * h / t);
        t += clamp(h, 0.03, 0.4);
        if (res < 0.01 || t > 8.0) { break; }
    }
    return clamp(res, 0.0, 1.0);
}

float occlusion(vec3 p, vec3 n) {
    float occ = 0.0;
    float w = 1.0;
    for (int i = 1; i <= 5; i++) {
        float h = 0.03 + 0.12 * float(i);
        occ += (h - map(p + n * h)) * w;
        w *= 0.7;
    }
    return clamp(1.0 - 1.8 * occ, 0.0, 1.0);
}

vec4 renderMain(void) {
    float z = fly * 2.0 + syn_BassTime * 0.8;
    vec3 ro = vec3(0.0, 0.4 + 0.05 * sin(TIME), z);
    vec3 forward = normalize(vec3(look.x, look.y, 1.0));
    vec3 right = normalize(cross(vec3(0.0, 1.0, 0.0), forward));
    vec3 up = cross(forward, right);
    vec3 rd = normalize(_uvc.x * right + _uvc.y * up + 1.4 * forward);

    float t = 0.0;
    bool hit = false;
    for (int i = 0; i < 110; i++) {
        float d = map(ro + rd * t);
        if (d < 0.0008 * t) { hit = true; break; }
        t += d;
        if (t > 40.0) { break; }
    }

    vec3 fogCol = mix(vec3(0.10, 0.11, 0.16), light_color * 0.6, clamp(rd.y * 1.5 + 0.2, 0.0, 1.0)) + light_color * 0.05 * syn_Level;
    vec3 col = fogCol;
    if (hit) {
        vec3 p = ro + rd * t;
        vec3 n = normalAt(p);
        vec3 l = normalize(vec3(0.35, 0.9, 0.25));
        float diff = max(dot(n, l), 0.0);
        float sh = shadows > 0.5 ? softShadow(p + n * 0.01, l) : 1.0;
        float ao = occlusion(p, n);
        vec3 albedo = mix(vec3(0.55, 0.5, 0.45), vec3(0.25, 0.22, 0.2), step(0.5, fract(p.z * 0.25)));
        col = albedo * (light_color * diff * sh * (1.6 + syn_BassHits) + vec3(0.25, 0.3, 0.42) * ao * (0.5 + 0.5 * n.y));
        // Glowing seams in the floor that pulse with the beat.
        float seam = smoothstep(0.03, 0.0, abs(fract(p.z * 0.5) - 0.5)) * step(p.y, -0.99);
        col += seam * light_color * syn_BPMSin * 1.5;
        col = mix(col, fogCol, 1.0 - exp(-0.0012 * t * t));
    }
    return vec4(_gamma(col, 1.3), 1.0);
}
