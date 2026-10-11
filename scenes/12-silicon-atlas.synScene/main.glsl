// ============================================================================
// Scene:        12-silicon-atlas.synScene
// Description:  Animated PCB / GPU package technical visualization.
//               Original fictional "GTX 5090 Ti" board; not a literal PCB layout.
// License:      MIT
// ============================================================================

float hash21(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

float boxSdf(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

float lineSeg(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a;
    vec2 ba = b - a;
    float h = clamp(dot(pa, ba) / max(dot(ba, ba), 0.00001), 0.0, 1.0);
    return length(pa - ba * h);
}

float tracePath(vec2 p, vec2 a, vec2 b, vec2 c, float w) {
    float d = min(lineSeg(p, a, b), lineSeg(p, b, c));
    return 1.0 - smoothstep(w, w + 0.0018, d);
}

float traceGlow(vec2 p, vec2 a, vec2 b, vec2 c, float w) {
    float d = min(lineSeg(p, a, b), lineSeg(p, b, c));
    return exp(-d * 70.0) * smoothstep(0.0, 0.3, w / max(d, 0.0001));
}

vec3 boardTrace(vec2 p) {
    vec3 c = vec3(0.0);
    // Dense, angular copper routing with occasional vias.
    for (int i = 0; i < 12; i++) {
        float fi = float(i);
        float y = -0.43 + fi * 0.078;
        float x = -0.76 + hash21(vec2(fi, 4.2)) * 0.22;
        vec2 a = vec2(-0.95, y);
        vec2 b = vec2(x, y);
        vec2 d = vec2(x + 0.05, y + 0.045 * (mod(fi, 2.0) * 2.0 - 1.0));
        vec2 e = vec2(0.22 + hash21(vec2(fi, 8.1)) * 0.18, d.y);
        float tr = tracePath(p, a, b, d, 0.0024);
        tr = max(tr, 1.0 - smoothstep(0.0024, 0.004, lineSeg(p, d, e)));
        float pulse = fract(TIME * (0.17 + fi * 0.012) + fi * 0.173);
        float along = length(p - mix(a, e, pulse));
        float bit = exp(-along * 95.0) * (0.45 + 0.55 * sin(TIME * 8.0 + fi));
        c += tr * vec3(0.025, 0.24, 0.31);
        c += bit * vec3(0.08, 0.8, 1.0);
    }
    for (int i = 0; i < 10; i++) {
        float fi = float(i);
        float x = -0.55 + fi * 0.12;
        vec2 a = vec2(x, -0.62);
        vec2 b = vec2(x, -0.53 + hash21(vec2(fi, 2.0)) * 0.1);
        vec2 d = vec2(x + 0.08, b.y + 0.05);
        float tr = tracePath(p, a, b, d, 0.002);
        c += tr * vec3(0.02, 0.17, 0.23);
        float pulse = fract(TIME * 0.23 + fi * 0.11);
        c += exp(-length(p - mix(a, d, pulse)) * 100.0) * vec3(0.05, 0.7, 1.0);
    }
    return c;
}

vec3 chip(vec2 p, vec2 center, vec2 halfSize, float rotation, float seed) {
    float cs = cos(rotation), sn = sin(rotation);
    vec2 q = p - center;
    q = mat2(cs, -sn, sn, cs) * q;
    float d = boxSdf(q, halfSize, 0.012);
    float body = 1.0 - smoothstep(0.0, 0.004, d);
    float edge = 1.0 - smoothstep(0.002, 0.006, abs(d));
    vec3 col = vec3(0.012, 0.026, 0.04) * body;
    col += edge * vec3(0.08, 0.36, 0.46);
    // Package substrate hatch.
    float hatch = step(0.86, hash21(floor((q + 1.0) * 130.0) + seed));
    float inner = 1.0 - smoothstep(0.0, 0.003, boxSdf(q, halfSize - 0.012, 0.003));
    col += hatch * inner * body * vec3(0.035, 0.09, 0.12);
    // Pins on all four edges.
    for (int i = 0; i < 9; i++) {
        float u = (float(i) - 4.0) / 5.0;
        float pinX = 1.0 - smoothstep(0.002, 0.004, abs(abs(q.x) - halfSize.x - 0.007));
        float pinY = 1.0 - smoothstep(0.002, 0.004, abs(abs(q.y) - halfSize.y - 0.007));
        float px = 1.0 - smoothstep(0.003, 0.006, abs(q.y - u * halfSize.y * 1.65));
        float py = 1.0 - smoothstep(0.003, 0.006, abs(q.x - u * halfSize.x * 1.65));
        col += (pinX * px + pinY * py) * vec3(0.16, 0.34, 0.39);
    }
    // Fine die grid, visible through the package window.
    float grid = max(1.0 - smoothstep(0.001, 0.003, abs(fract(q.x * 90.0) - 0.5) / 90.0),
                     1.0 - smoothstep(0.001, 0.003, abs(fract(q.y * 90.0) - 0.5) / 90.0));
    float die = 1.0 - smoothstep(0.0, 0.006, boxSdf(q, halfSize * 0.53, 0.004));
    col += grid * die * body * vec3(0.025, 0.13, 0.16);
    col += die * body * (0.015 + 0.012 * sin(TIME * 2.2 + seed)) * vec3(0.3, 0.9, 1.0);
    return col;
}

vec3 passive(vec2 p, vec2 center, vec2 size, float seed) {
    vec2 q = p - center;
    float d = boxSdf(q, size, 0.003);
    float body = 1.0 - smoothstep(0.0, 0.003, d);
    float rim = 1.0 - smoothstep(0.001, 0.003, abs(d));
    vec3 c = body * vec3(0.045, 0.065, 0.075);
    c += rim * vec3(0.17, 0.27, 0.3);
    float metal = 1.0 - smoothstep(0.001, 0.004, abs(abs(q.x) - size.x * 0.72));
    c += metal * body * vec3(0.18, 0.27, 0.3);
    c += body * 0.025 * sin(seed + TIME * 1.5) * vec3(0.2, 0.7, 1.0);
    return c;
}

vec3 via(vec2 p, vec2 center, float radius) {
    float d = length(p - center);
    float ring = 1.0 - smoothstep(0.0015, 0.003, abs(d - radius));
    float core = 1.0 - smoothstep(radius * 0.28, radius * 0.55, d);
    return ring * vec3(0.09, 0.31, 0.38) + core * vec3(0.015, 0.045, 0.055);
}

vec4 renderMain(void) {
    vec2 p = _uvc;
    // Board plane, subtly tilted with perspective-like shear.
    p.x += p.y * 0.12;
    p *= 1.0 + 0.035 * p.y;
    float t = TIME;
    vec3 col = vec3(0.004, 0.009, 0.018);

    // Technical blueprint background and coordinate grid.
    vec2 gridP = p * vec2(42.0, 32.0);
    vec2 gridF = abs(fract(gridP) - 0.5);
    float grid = max(1.0 - smoothstep(0.475, 0.49, gridF.x),
                     1.0 - smoothstep(0.475, 0.49, gridF.y));
    col += grid * 0.018 * vec3(0.18, 0.42, 0.52);
    float microGrid = max(1.0 - smoothstep(0.48, 0.495, abs(fract(p.x * 160.0) - 0.5)),
                          1.0 - smoothstep(0.48, 0.495, abs(fract(p.y * 120.0) - 0.5)));
    col += microGrid * 0.006 * vec3(0.3, 0.5, 0.6);

    // PCB silhouette and bevel.
    float boardD = boxSdf(p, vec2(0.82, 0.68), 0.055);
    float board = 1.0 - smoothstep(0.0, 0.012, boardD);
    float bevel = 1.0 - smoothstep(0.004, 0.014, abs(boardD));
    col = mix(col, vec3(0.009, 0.026, 0.035), board);
    col += bevel * vec3(0.035, 0.18, 0.21);
    col += board * boardTrace(p);

    // Large central GPU package and die.
    col += chip(p, vec2(0.04, 0.02), vec2(0.205, 0.19), -0.035, 3.1);
    float dieD = boxSdf(p - vec2(0.04, 0.02), vec2(0.135, 0.12), 0.008);
    float die = 1.0 - smoothstep(0.0, 0.004, dieD);
    col += die * vec3(0.025, 0.09, 0.12);
    float scan = exp(-abs(fract((p.y + t * 0.11) * 3.5) - 0.5) * 100.0);
    col += die * scan * vec3(0.03, 0.2, 0.27);
    // GPU data lanes leaving the die.
    for (int i = 0; i < 14; i++) {
        float fi = float(i);
        float yy = -0.095 + fi * 0.014;
        float pulse = fract(t * (0.25 + fi * 0.013) + fi * 0.071);
        float d = lineSeg(p, vec2(0.18, yy), vec2(0.38, yy + 0.025));
        col += exp(-d * 230.0) * vec3(0.02, 0.22, 0.3);
        col += exp(-length(p - vec2(mix(0.18, 0.38, pulse), yy + 0.025 * pulse)) * 100.0)
             * vec3(0.08, 0.8, 1.0);
    }

    // VRAM packages in a symmetric ring around the processor.
    col += chip(p, vec2(-0.34, 0.20), vec2(0.075, 0.052), 0.0, 1.0);
    col += chip(p, vec2(-0.34, 0.04), vec2(0.075, 0.052), 0.0, 2.0);
    col += chip(p, vec2(-0.34, -0.12), vec2(0.075, 0.052), 0.0, 3.0);
    col += chip(p, vec2(0.37, 0.23), vec2(0.075, 0.052), 0.0, 4.0);
    col += chip(p, vec2(0.38, 0.08), vec2(0.075, 0.052), 0.0, 5.0);
    col += chip(p, vec2(0.38, -0.08), vec2(0.075, 0.052), 0.0, 6.0);
    col += chip(p, vec2(0.19, -0.39), vec2(0.075, 0.045), 0.0, 7.0);
    col += chip(p, vec2(0.00, -0.42), vec2(0.075, 0.045), 0.0, 8.0);

    // VRM power stages, inductors and capacitors.
    for (int i = 0; i < 8; i++) {
        float fi = float(i);
        float x = -0.52 + fi * 0.105;
        col += passive(p, vec2(x, 0.49), vec2(0.031, 0.048), fi);
        col += passive(p, vec2(x, 0.37), vec2(0.021, 0.022), fi + 10.0);
        col += via(p, vec2(x, 0.29), 0.006);
    }
    // Small controller ICs and clock generator.
    col += chip(p, vec2(-0.60, -0.35), vec2(0.055, 0.045), 0.0, 12.0);
    col += chip(p, vec2(0.59, -0.35), vec2(0.055, 0.045), 0.0, 13.0);
    col += chip(p, vec2(-0.59, 0.06), vec2(0.038, 0.03), 0.0, 14.0);
    col += chip(p, vec2(0.60, 0.10), vec2(0.038, 0.03), 0.0, 15.0);

    // PCIe edge connector, gold fingers and mounting holes.
    float edgeD = boxSdf(p - vec2(0.0, -0.635), vec2(0.56, 0.025), 0.006);
    col += (1.0 - smoothstep(0.0, 0.003, edgeD)) * vec3(0.16, 0.12, 0.045);
    for (int i = 0; i < 32; i++) {
        float x = -0.52 + float(i) * 0.0335;
        float finger = 1.0 - smoothstep(0.002, 0.004, boxSdf(p - vec2(x, -0.633), vec2(0.009, 0.016), 0.002));
        col += finger * vec3(0.55, 0.36, 0.11);
    }
    col += via(p, vec2(-0.72, 0.57), 0.014);
    col += via(p, vec2(0.72, 0.57), 0.014);
    col += via(p, vec2(-0.72, -0.55), 0.014);
    col += via(p, vec2(0.72, -0.55), 0.014);

    // Power delivery buses: broad glowing lanes, animated current packets.
    for (int i = 0; i < 5; i++) {
        float fi = float(i);
        vec2 a = vec2(-0.54, 0.30 - fi * 0.035);
        vec2 b = vec2(-0.23, 0.22 - fi * 0.028);
        vec2 c = vec2(-0.12, 0.12 - fi * 0.022);
        float d = min(lineSeg(p, a, b), lineSeg(p, b, c));
        col += exp(-d * 100.0) * vec3(0.025, 0.12, 0.17);
        float packet = fract(t * (0.18 + fi * 0.035) + fi * 0.2);
        col += exp(-length(p - mix(a, c, packet)) * 115.0) * vec3(0.06, 0.65, 0.95);
    }

    // Oscilloscope sweep and a restrained cyan emissive vignette.
    float sweep = exp(-abs(p.x - (fract(t * 0.075) * 1.8 - 0.9)) * 100.0);
    col += sweep * board * vec3(0.008, 0.035, 0.05);
    float vignette = 1.0 - 0.38 * dot(p, p);
    col *= max(vignette, 0.35);
    col = pow(max(col, 0.0), vec3(0.88));
    return vec4(col, 1.0);
}
