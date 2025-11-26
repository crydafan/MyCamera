// https://casual-effects.com/research/McGuire2009Bayer/bayer-jgt09.pdf
RWTexture2D<half> RawNormalized;
RWTexture2D<half4> Rgba;

[push_constant]
cbuffer Uniforms { int2 size; }

static const float4 kA = float4(-1.0, -1.5, 0.5, -1.0) / 8.0;
static const float4 kB = float4(2.0, 0.0, 0.0, 4.0) / 8.0;
static const float4 kC = float4(4.0, 6.0, 5.0, 5.0) / 8.0;
static const float4 kD = float4(0.0, 2.0, -1.0, -1.0) / 8.0;
static const float4 kE = kA.xywz;
static const float4 kF = kB.xywz;

uint2 safeCoord(int x, int y) {
  int2 coords = int2(x, y);
  uint2 safe = clamp(coords, uint2(0, 0), size - uint2(1, 1));
  return safe;
}

[Shader("compute")]
[NumThreads(8, 8, 1)]
void computeMain(uint3 threadId: SV_DispatchThreadID) {
  uint x = threadId.x;
  uint y = threadId.y;

  uint xPhase = x & 1;
  uint yPhase = y & 1;

  float A =
      RawNormalized[safeCoord(x, y + -2)] + RawNormalized[safeCoord(x, y + 2)];
  float B =
      RawNormalized[safeCoord(x, y + -1)] + RawNormalized[safeCoord(x, y + 1)];
  float C = RawNormalized[safeCoord(x, y)];
  float D = RawNormalized[safeCoord(x - 1, y - 1)] +
            RawNormalized[safeCoord(x + 1, y - 1)] +
            RawNormalized[safeCoord(x - 1, y + 1)] +
            RawNormalized[safeCoord(x + 1, y + 1)];
  float E =
      RawNormalized[safeCoord(x - 2, y)] + RawNormalized[safeCoord(x + 2, y)];
  float F =
      RawNormalized[safeCoord(x - 1, y)] + RawNormalized[safeCoord(x + 1, y)];

  // x cross
  // y checker
  // z theta
  // w phi
  float4 pattern = kA * A + kB * B + kC * C + kD * D + kE * E + kF * F;

  Rgba[int2(x, y)] =
      (yPhase == 0)
          ? ((xPhase == 0)
                 ? half4(half(C), half(pattern.x), half(pattern.y), 1.0h)
                 : half4(half(pattern.z), half(C), half(pattern.w), 1.0h))
          : ((xPhase == 0)
                 ? half4(half(pattern.w), half(C), half(pattern.z), 1.0h)
                 : half4(half(pattern.y), half(pattern.x), half(C), 1.0h));
}
