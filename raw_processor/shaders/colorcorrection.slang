RWTexture2D<half4> Rgb;

[push_constant]
cbuffer Uniforms {
  // row_major float3x3 forwardMatrix1;
  // row_major float3x3 forwardMatrix2;
  row_major float3x3 colorCorrectionTransform;
  // float3 neutralPoint;
}

static const float3x3 M_XYZ_to_sRGB = float3x3(
    3.2406, -1.5372, -0.4986, -0.9689, 1.8758, 0.0415, 0.0557, -0.2040, 1.0570);

[Shader("compute")]
[NumThreads(8, 8, 1)]
void computeMain(uint3 threadId: SV_DispatchThreadID) {
  uint2 coordinates = threadId.xy;

  float3 raw = Rgb[coordinates].rgb;

  // raw[0] *= 1.0 / neutralPoint[0];
  // raw[1] *= 1.0 / neutralPoint[1];
  // raw[2] *= 1.0 / neutralPoint[2];
  // raw *= 1.0 / neutralPoint;

  // float3 XYZ = mul(forwardMatrix1, raw);
  // float3 sRGB = mul(M_XYZ_to_sRGB, XYZ);

  float3 sRGB = mul(colorCorrectionTransform, raw);

  Rgb[coordinates] = half4(half3(sRGB), 1.0h);
}
