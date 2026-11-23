#version 460 core

#include <flutter/runtime_effect.glsl>

// A room picture shifted by its depth map: near things move more than far
// ones as the buyer drags, so the room has real depth.

uniform vec2 uSize;      // the box, in pixels
uniform vec2 uLook;      // -1..1 on each axis
uniform float uStrength; // largest shift, as a share of the picture
uniform vec2 uImage;     // the room picture's size, in pixels
uniform sampler2D uRoom;
uniform sampler2D uDepth;

out vec4 fragColor;

void main() {
  vec2 frag = FlutterFragCoord().xy;
  // Cover the box, then zoom 5% so a shifted edge never shows.
  float scale = max(uSize.x / uImage.x, uSize.y / uImage.y) * 1.05;
  vec2 uv = ((frag - uSize * 0.5) / scale + uImage * 0.5) / uImage;
  float depth = texture(uDepth, clamp(uv, 0.0, 1.0)).r;
  vec2 shifted = clamp(uv + uLook * uStrength * (depth - 0.5), 0.0, 1.0);
  fragColor = texture(uRoom, shifted);
}
