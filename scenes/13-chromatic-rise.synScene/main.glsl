// Chromatic Rise — original Strobe interpretation of the owner's
// TouchDesigner video: https://youtu.be/w0JcYtmRr0U
// Nested flowing noise, prismatic contours and screen-space relief.
// No video samples, external textures or feedback history required.

vec4 renderMain(void) {
    float audio = clamp(syn_BassLevel, 0.0, 1.0) * reactivity;
    float t = TIME * speed * 0.16;
    vec2 p = _uvc * scale;
    p += vec2(0.22 * sin(t * 0.7), -t * 0.22);

    vec2 drift = vec2(t * 0.29, -t * 0.21);
    vec2 q = vec2(_fbm(p + drift, 2.0),
                  _fbm(p + vec2(7.8, 3.1) - drift, 2.0));
    vec2 w = p + (warp + audio * 0.35) * (q - 0.5);
    vec2 r = vec2(_fbm(w + vec2(1.9, 8.2) + drift * 0.4, 2.0),
                  _fbm(w + vec2(8.3, 2.8) - drift * 0.6, 2.0));
    vec2 marble = w + warp * (r - 0.5);
    float field = _fbm(marble, 3.0);

    // Fine folded strata ride on the larger, slowly changing islands.
    float veins = sin((marble.x + marble.y * 0.65 + field * 3.0) * detail);
    float height = field + 0.018 * veins;
    float phase = height * bands + r.x * 1.3 + q.y * 0.7;
    vec3 color = 0.5 + 0.5 * cos(6.2831853 *
        (phase + hue + t * color_speed * 0.12 + vec3(0.0, 0.33, 0.67)));
    color = mix(vec3(dot(color, vec3(0.2126, 0.7152, 0.0722))), color, saturation);
    float pearlescence = smoothstep(0.48, 0.73, q.x) * 0.52;
    color = mix(color, vec3(0.38, 0.82, 0.78) + color * 0.35, pearlescence);

    // Derivatives give embossed edges without extra noise evaluations.
    vec2 slope = vec2(dFdx(height), dFdy(height)) * RENDERSIZE.y / scale;
    vec3 normal = normalize(vec3(-slope * relief, 1.0));
    vec3 light = normalize(vec3(-0.55, 0.65, 0.8));
    float diffuse = max(dot(normal, light), 0.0);
    float shine = pow(max(dot(reflect(-light, normal), vec3(0.0, 0.0, 1.0)), 0.0), 18.0);
    float ridge = abs(sin(phase * 6.2831853));
    float aa = max(fwidth(phase) * 6.2831853, 0.015);
    float outline = 1.0 - smoothstep(0.08, 0.08 + aa, ridge);
    color *= 0.72 + 0.45 * diffuse;
    color = mix(color, color * 0.18, outline * edge);
    color += vec3(0.6, 0.9, 1.0) * shine * 0.32;
    color *= 1.0 + audio * 0.15;
    return vec4(clamp(color, 0.0, 1.0), 1.0);
}
