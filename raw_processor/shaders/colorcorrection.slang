RWTexture2D<half4> Rgba;

[push_constant]
cbuffer Uniforms {
  float3x3 forwardMatrix1;
  float3x3 forwardMatrix2;
}

static const float3x3 M_XYZ_to_sRGB = float3x3(
    3.2406, -1.5372, -0.4986, -0.9689, 1.8758, 0.0415, 0.0557, -0.2040, 1.0570);

[Shader("compute")]
[NumThreads(8, 8, 1)]
void computeMain(uint3 threadId: SV_DispatchThreadID) {
  uint2 coordinates = threadId.xy;

  float3 RGB = Rgba[coordinates].rgb;

  float3 XYZ = mul(RGB, forwardMatrix1);

  float3 sRGB = mul(M_XYZ_to_sRGB, XYZ);

  Rgba[coordinates] = half4(half3(sRGB), 1.0h);
}
