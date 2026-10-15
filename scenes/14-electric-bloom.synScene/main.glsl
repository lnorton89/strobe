// Electric Bloom: original procedural floral scene for Strobe, rebuilt from
// the owner's floral reference (the image itself is not bundled).
//
// Three depth layers of inked, glowing flowers over black, back to front:
// small dim ones far away, mid-sized ones, and a few big soft foreground
// blooms. Each flower has two rings of petals that unfurl and fold as it
// opens, dark inked veins and rims, and a beaded pollen centre. Drifting
// pollen specks float between the layers. Cells are rotated and jittered per
// layer, so no grid shows.
//
// Two passes: a one-texel clock integrates the sway speed so mids can make
// the garden stir without the pattern jumping, then the garden is drawn.

float flowRate() {
    return speed * (1.0 + 0.8 * clamp(syn_MidLevel, 0.0, 1.0) * reactivity);
}

float stepDt(float prevTime) {
    float d = TIME - prevTime;
    return (d > 0.0 && d < 0.2) ? d : 1.0 / 60.0;
}

vec4 clockPass() {
    vec4 prev = texelFetch(clock, ivec2(0), 0);
    // A cleared target (first frame, or a resize) restarts at the scene time.
    if (prev.x <= 0.0) { return vec4(TIME, TIME * flowRate(), 0.0, 1.0); }
    float dt = stepDt(prev.x);
    return vec4(TIME, prev.y + dt * flowRate(), 0.0, 1.0);
}

// Vivid colour from a hue, pushed toward full saturation.
vec3 vivid(float h) {
    return clamp(0.5 + 0.66 * cos(6.2831853 * (h + vec3(0.0, 0.33, 0.67))), 0.0, 1.0);
}

// A dark line through the middle of each petal and two fainter ones beside
// it; widths are in petal-angle units, so they thin out toward the tips.
float veins(float phi, float rho, float n, float aaR) {
    float w = (0.016 + aaR) * n / (6.2831853 * max(rho, 0.18));
    float wa = aaR * n / (6.2831853 * max(rho, 0.18)) + 0.002;
    float a = abs(phi);
    float mid = 1.0 - smoothstep(w - wa, w + wa, a);
    float side = 1.0 - smoothstep(w * 0.7 - wa, w * 0.7 + wa, abs(a - 0.27));
    return clamp(mid + 0.55 * side * smoothstep(0.35, 0.7, rho), 0.0, 1.0);
}

// Signed distance (flower-radius units, positive inside) to a ring of petals
// whose outline is a cosine lobe; pw returns the lobe profile for shading.
float petalSD(float th, float n, float len, float m, float shp, float rho, out float edge) {
    float l = 0.5 + 0.5 * cos(n * th);
    float pw = pow(l, shp);
    edge = len * (m + (1.0 - m) * pw);
    float slope = len * (1.0 - m) * shp * pw / max(l, 0.03) * (-0.5 * n * sin(n * th));
    return (edge - rho) / sqrt(1.0 + slope * slope / (rho * rho + 0.02));
}

// One flower, d the offset from its centre in cell units. Returns colour in
// col, coverage in a, and the glow it adds to the black around it in glowc.
// z is its stacking order within the layer.
void flower(vec2 d, vec3 hv, vec2 id, float layer, float T, float bass, float high, float kick,
            float pxc, float aaMul, out vec3 col, out float a, out vec3 glowc,
            out float z) {
    col = vec3(0.0);
    a = 0.0;
    glowc = vec3(0.0);
    vec2 hw = _hash22(id * 1.37 + layer * 7.1);
    z = hw.x;

    // Opening: each flower follows its own slow cycle; bass pushes it open.
    float cyc = 0.5 + 0.5 * sin(T * 0.21 * (0.7 + 0.6 * hw.y) + hv.x * 6.2831853 + layer * 1.7);
    cyc = cyc * cyc * (3.0 - 2.0 * cyc);
    float o = clamp(mix(0.12, 1.0, cyc) * bloom + 0.38 * bass * reactivity, 0.0, 1.0);
    float R = (0.26 + 0.20 * hv.x) * (0.50 + 0.50 * o);
    float dl = length(d);
    // The petals' reach can pass the radius a little while the flower pulses.
    float rho = dl / R;

    // A curling stem down from the flower, behind it: ink outline, then a
    // dark leaf-green core. It stays inside the 0.78-cell reach of the flower.
    float hue0 = hue + 0.88 + 0.95 * pow(hv.x, 1.35) + layer * 0.07 + 0.10 * kick;
    vec3 V0 = vivid(hue0);
    if (stems > 0.01 && layer > 0.5 && d.y < 0.0) {
        float yl = -d.y;
        float len = 0.30 + 0.30 * hw.y;
        if (yl < len + R * 0.4) {
            float xs = 0.09 * sin(yl * 6.0 + hv.z * 20.0 + T * 0.4 * sway_amt) * (yl / len);
            float w = (0.0085 + 0.004 * (1.0 - yl / len)) * (0.6 + 0.3 * layer);
            float dd = abs(d.x - xs);
            float aaS = pxc * 1.2;
            float fadeEnd = 1.0 - smoothstep(len - 0.05, len, yl);
            float ink = (1.0 - smoothstep(w * 1.5 - aaS, w * 1.5 + aaS, dd)) * fadeEnd;
            float core = (1.0 - smoothstep(w * 0.55 - aaS, w * 0.55 + aaS, dd));
            vec3 sc = mix(vec3(0.0), vivid(hue0 + 0.38) * 0.55, core);
            col = sc;
            a = ink * stems;
        }
    }
    if (rho > min(1.75, 0.78 / R)) { return; }

    vec3 V1 = vivid(hue0 + 0.15);
    vec3 V2 = vivid(hue0 - 0.10);
    float sway = 0.20 * sin(T * 0.5 + hv.z * 20.0 + layer) * sway_amt;
    float th = atan(d.y, d.x) + hv.y * 6.2831853 + sway + T * 0.02 * (hv.x - 0.5);
    float n1 = 6.0 + floor(hw.y * 4.0);
    float aaR = pxc / R * 1.4 * aaMul;

    float wide = petal_width;
    float shp = mix(3.2, 0.9, clamp((wide - 0.6) / 0.9, 0.0, 1.0));

    // Three rings of petals, back to front. sd is an approximate signed
    // distance to the outline in flower-radius units (positive inside), so
    // the ink rim follows every side of a petal and the edge antialiases at
    // any size.
    float aN = n1 * th / 6.2831853;
    float edgeZ;
    float sdZ = petalSD(th + 1.5707963 / n1, n1, 1.0 * (0.9 + 0.1 * o), 0.46, shp * 1.15, rho, edgeZ);
    float covZ = smoothstep(-aaR, aaR, sdZ);
    float edgeA;
    float sdA = petalSD(th, n1, 0.84 * (0.88 + 0.12 * o), 0.52, shp, rho, edgeA);
    float covA = smoothstep(-aaR, aaR, sdA);
    float thB = th + 3.14159265 / n1;
    float edgeB;
    float sdB = petalSD(thB, n1, 0.60 * (0.55 + 0.45 * o), 0.50, shp * 0.85, rho, edgeB);
    float covB = smoothstep(-aaR, aaR, sdB);

    float coreR = 0.17 * (0.7 + 0.3 * o);
    float covC = 1.0 - smoothstep(coreR - aaR, coreR + aaR, rho);

    // Back petals: deeper and cooler, seen between the front ones.
    if (covZ > 0.0) {
        float phiZ = fract(n1 * (th + 1.5707963 / n1) / 6.2831853 + 0.5) - 0.5;
        float tZ = clamp(rho / max(edgeZ, 0.05), 0.0, 1.0);
        vec3 cZ = mix(V0, V2 * 0.8, smoothstep(0.2, 1.0, tZ));
        cZ *= 0.50 + 0.30 * cos(phiZ * 3.14159265 * 1.1);
        cZ *= 1.0 - linework * (0.75 * (1.0 - smoothstep(0.0, 0.05 + aaR, sdZ)) + 0.5 * veins(phiZ, rho, n1, aaR));
        col = mix(col, cZ, covZ);
        a = max(a, covZ);
    }

    // Outer petals: warm throat to saturated tip, inked veins and rim.
    if (covA > 0.0) {
        float phiA = fract(aN + 0.5) - 0.5;
        float tA = clamp(rho / max(edgeA, 0.05), 0.0, 1.0);
        vec3 cA = mix(V1 * 1.12, V0, smoothstep(0.15, 0.95, tA));
        cA *= 0.60 + 0.40 * cos(phiA * 3.14159265 * 1.1);
        cA = mix(cA, vec3(1.0, 0.95, 0.8), 0.45 * (1.0 - smoothstep(0.0, 0.42, tA)));
        float vA = veins(phiA + 0.05 * sin(rho * 9.0 + hv.y * 20.0), rho, n1, aaR);
        float rimA = smoothstep(0.0, 0.05 + aaR, sdA);
        cA *= 1.0 - linework * (0.82 * vA * smoothstep(0.12, 0.3, rho) + 0.85 * (1.0 - rimA));
        col = mix(col, cA, covA);
        a = max(a, covA);
    }

    // Inner petals: hotter, brighter.
    if (covB > 0.0) {
        float aB = n1 * thB / 6.2831853;
        float phiB = fract(aB + 0.5) - 0.5;
        float tB = clamp(rho / max(edgeB, 0.05), 0.0, 1.0);
        vec3 cB = mix(V1 * 1.3, V1 * 1.05, smoothstep(0.2, 0.95, tB));
        cB *= 0.65 + 0.35 * cos(phiB * 3.14159265 * 1.1);
        cB = mix(cB, vec3(1.0, 0.96, 0.82), 0.35 * (1.0 - tB));
        float vB = veins(phiB, rho, n1, aaR);
        float rimB = smoothstep(0.0, 0.05 + aaR, sdB);
        cB *= 1.0 - linework * (0.78 * vB * smoothstep(0.12, 0.3, rho) + 0.9 * (1.0 - rimB));
        col = mix(col, cB, covB);
        a = max(a, covB);
    }

    // Pollen centre: a bright disc of tiny beads on dark ink.
    if (covC > 0.0) {
        float ring = floor(rho / coreR * 5.0);
        float bead = length(vec2(fract(rho / coreR * 5.0) - 0.5,
                                 (fract(th * (4.0 + ring * 2.5) / 6.2831853 + ring * 0.37) - 0.5) * 0.9));
        float beads = 1.0 - smoothstep(0.30, 0.42 + aaR * 4.0, bead);
        float twinkle = 0.65 + 0.35 * sin(T * 4.0 + ring * 2.1 + hv.z * 30.0) + 1.2 * high * hw.y;
        vec3 cC = mix(vec3(0.02, 0.01, 0.02), mix(vec3(1.0, 0.8, 0.15), vec3(1.0, 0.97, 0.7), 0.5 * beads) * twinkle, beads);
        col = mix(col, cC, covC);
        a = max(a, covC);
    }

    // Glow: a wide halo in the petal colour and a hot centre.
    vec3 halo = V0;
    float g1 = exp(-rho * rho * 2.6) * (0.28 + 0.30 * o);
    float g2 = exp(-rho * rho * 30.0) * (0.5 + 1.1 * bass * reactivity + 0.8 * kick);
    glowc = (halo * g1 + vec3(1.0, 0.78, 0.35) * g2) * glow;
}

vec4 garden(void) {
    float T = texelFetch(clock, ivec2(0), 0).y;
    float bass = clamp(syn_BassLevel, 0.0, 1.0);
    float high = clamp(syn_HighLevel, 0.0, 1.0) * reactivity;
    float kick = clamp(syn_BassHits, 0.0, 1.0) * reactivity;
    vec2 p = _uvc;
    p *= 1.0 - 0.035 * bass * reactivity;
    float pxs = 1.0 / RENDERSIZE.y;                 // one pixel, in screen heights

    vec3 col = vec3(0.0);
    for (int L = 0; L < 3; L++) {
        float fl = float(L);                        // 0 back, 1 middle, 2 front
        float S = (L == 0 ? 3.7 : (L == 1 ? 2.3 : 1.3)) * density;
        float ang = (L == 0 ? 0.42 : (L == 1 ? -0.27 : 0.16));
        float occ = (L == 0 ? 0.78 : (L == 1 ? 0.68 : 0.46));
        float bright = (L == 0 ? 0.42 : (L == 1 ? 0.78 : 1.0));
        float aaMul = (L == 0 ? 2.6 : (L == 1 ? 1.0 : 4.5));
        float c = cos(ang);
        float s = sin(ang);
        vec2 q = mat2(c, -s, s, c) * p * S;
        // Parallax drift: the near layer slides faster than the far one.
        q += vec2(0.11, 0.07 + 0.05 * fl) * (0.4 + 0.6 * fl) * T * 0.35 + vec2(7.3 * fl, 2.9 * fl);
        // A slow wobble bends the arrangement away from any lattice.
        q += 0.18 * vec2(sin(q.y * 0.9 + T * 0.13 + fl), sin(q.x * 0.8 - T * 0.11 + 2.0 * fl));
        vec2 id0 = floor(q);
        vec2 f = fract(q);
        vec2 sgn = vec2(f.x < 0.5 ? -1.0 : 1.0, f.y < 0.5 ? -1.0 : 1.0);
        float pxc = pxs * S;

        vec3 lc = vec3(0.0);
        float la = 0.0;
        float lz = -1.0;
        vec3 lg = vec3(0.0);
        for (int k = 0; k < 4; k++) {
            vec2 off = vec2(float(k - 2 * (k / 2)), float(k / 2)) * sgn;
            vec2 id = id0 + off;
            vec3 hv = _hash32(id + fl * 31.7);
            if (hv.z > occ) { continue; }
            vec2 cen = off + 0.5 + (hv.xy - 0.5) * 0.44;
            vec2 dv = f - cen;
            if (dot(dv, dv) > 0.6084) { continue; }
            vec3 fc;
            float fa;
            vec3 fg;
            float fz;
            flower(dv, hv, id, fl, T, bass, high, kick, pxc, aaMul, fc, fa, fg, fz);
            lg += fg;
            if (fa > 0.0) {
                if (fz > lz) { lc = mix(lc, fc, fa); la = max(la, fa); lz = fz; }
                else { lc = mix(fc, lc, la); la = max(la, fa); }
            }
        }
        // The back layers sink into a cool haze; glow always adds on top.
        vec3 tint = mix(vec3(1.0), vec3(0.55, 0.62, 1.0), (1.0 - bright) * 0.8);
        col = col * (1.0 - la) + lc * la * bright * tint;
        col += lg * bright;
    }

    // Drifting pollen: two sparse layers of soft specks.
    for (int P = 0; P < 2; P++) {
        float fp = float(P);
        float PS = (P == 0 ? 11.0 : 6.5);
        vec2 pq = p * PS + vec2(0.0, T * (0.14 + 0.1 * fp)) + vec2(13.7 * fp, 3.1);
        vec2 pid = floor(pq);
        vec2 pf = fract(pq);
        vec3 ph = _hash32(pid + fp * 17.0);
        vec2 pos = 0.5 + (ph.xy - 0.5) * 0.24 + 0.10 * vec2(sin(T * (0.5 + ph.z) + ph.x * 30.0),
                                                           cos(T * (0.4 + ph.y) + ph.z * 30.0));
        float rr = (P == 0 ? 0.055 : 0.075) * (0.6 + 0.8 * ph.z);
        float aap = pxs * PS * 1.5;
        float dist = length(pf - pos);
        float speck = (1.0 - smoothstep(rr - aap, rr + aap, dist)) * step(0.5, ph.x + 0.2 * fp);
        float halo = exp(-dist * dist / (rr * rr * 5.0)) * (1.0 - smoothstep(0.10, 0.28, dist)) * step(0.5, ph.x + 0.2 * fp);
        float tw = 0.45 + 0.55 * sin(T * 2.0 + ph.y * 40.0);
        float spark = step(0.80 - 0.4 * high, fract(ph.z * 91.0 + floor(T * 4.0 + ph.y * 9.0) * 0.37));
        float amount = pollen * (0.35 + 0.65 * tw + 1.6 * high * spark);
        vec3 pc = mix(vec3(1.0, 0.82, 0.4), vivid(hue + ph.x), 0.35);
        col += pc * (speck * 0.9 + halo * 0.18) * amount * (P == 0 ? 0.55 : 0.8);
    }

    // Soft shoulder: keeps saturation where glows overlap, black stays black.
    col = 1.0 - exp(-1.35 * col);
    return vec4(col * brightness, 1.0);
}

vec4 renderMain(void) {
    if (PASSINDEX == 0) { return clockPass(); }
    return garden();
}
