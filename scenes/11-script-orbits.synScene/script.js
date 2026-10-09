// Script Orbits — Strobe base library, scripted tier. MIT, original work.
//
// A small physics loop: four orbs, each pulled by a spring toward its slot
// on a rotating ring, and thrown off it by an impulse whenever the bass
// hits (or the Kick button is pressed). The positions go out through the
// orb1..orb4 controls, which main.glsl reads.
//
// What this exercises: setup(), update(dt), persistent JavaScript state,
// reading audio and control globals, the onOffToOn event on an audio hit
// and on a bang, setControl, and print().

var COUNT = 4;
var HALF_PI = Math.PI / 2;

// State persists between frames: this is the point of a script.
var x = [], y = [], vx = [], vy = [];
var phase = 0;
var kicks = 0;
var seed = 20260709;

// A small deterministic generator, so the scene moves the same way every
// run (Math.random would differ each time).
function rand() {
  seed = (seed * 1664525 + 1013904223) % 4294967296;
  return seed / 4294967296;
}

function setup() {
  for (var i = 0; i < COUNT; i++) {
    x[i] = Math.cos(i * HALF_PI) * 0.5;
    y[i] = Math.sin(i * HALF_PI) * 0.5;
    vx[i] = 0;
    vy[i] = 0;
  }
  // Fires when the bass hit crosses 0.5 going up, and when the bang goes
  // off. Both call the same function.
  onOffToOn("syn_BassHits", "onKick");
  onOffToOn("kick_orbs", "onKick");
}

function onKick(value, previous) {
  kicks += 1;
  var power = 0.9 + 2.6 * chaos;
  for (var i = 0; i < COUNT; i++) {
    var angle = rand() * 2 * Math.PI;
    var strength = power * (0.5 + rand());
    vx[i] += Math.cos(angle) * strength;
    vy[i] += Math.sin(angle) * strength;
  }
  if (kicks % 16 === 1) {
    print("orbit kick #" + kicks + " (chaos " + chaos.toFixed(2) + ")");
  }
}

function update(dt) {
  // Never integrate a long stall in one step: the springs would explode.
  var h = Math.min(dt, 1 / 30);

  phase += h * (0.5 + 1.5 * syn_MidLevel);
  var radius = 0.34 + 0.22 * syn_Level + 0.08 * syn_BPMSin;

  var k = 14 * pull;                    // spring stiffness
  var damping = 1.5 * Math.sqrt(k);     // a little under critical

  for (var i = 0; i < COUNT; i++) {
    var angle = phase + i * HALF_PI;
    var tx = Math.cos(angle) * radius;
    var ty = Math.sin(angle) * radius;

    vx[i] += ((tx - x[i]) * k - vx[i] * damping) * h;
    vy[i] += ((ty - y[i]) * k - vy[i] * damping) * h;
    x[i] += vx[i] * h;
    y[i] += vy[i] * h;

    // Bounce softly off the edge of the picture.
    if (x[i] > 1) { x[i] = 1; vx[i] = -Math.abs(vx[i]) * 0.5; }
    if (x[i] < -1) { x[i] = -1; vx[i] = Math.abs(vx[i]) * 0.5; }
    if (y[i] > 1) { y[i] = 1; vy[i] = -Math.abs(vy[i]) * 0.5; }
    if (y[i] < -1) { y[i] = -1; vy[i] = Math.abs(vy[i]) * 0.5; }

    setControl("orb" + (i + 1), x[i], y[i]);
  }
}
