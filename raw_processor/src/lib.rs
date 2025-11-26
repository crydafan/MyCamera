use std::{panic, slice};

use android_logger::Config;
use jni::{
    JNIEnv,
    objects::{JByteArray, JByteBuffer, JClass, JFloatArray, JIntArray},
    sys::{jbyte, jint, jlong},
};
use log::{LevelFilter, error, info};
use vulkano::VulkanLibrary;

mod pipeline;

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_mdnssknght_mycamera_processing_NativeRawProcessor_00024Companion_nativeInit(
    _: JNIEnv,
    _: JClass,
) -> jlong {
    android_logger::init_once(
        Config::default()
            .with_max_level(LevelFilter::Trace)
            .with_tag("RustNative"),
    );

    panic::set_hook(Box::new(move |panic_info| {
        if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            error!("panic occurred: {s:?}");
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            error!("panic occurred: {s:?}");
        } else {
            error!("panic occurred");
        }

        if let Some(location) = panic_info.location() {
            error!(
                "panic occurred in file '{}' at line {}",
                location.file(),
                location.line(),
            );
        } else {
            error!("panic occurred but can't get location information...");
        }
    }));

    info!("Hello, from Rust!");

    let library = VulkanLibrary::new().expect("Failed to find local Vulkan library");

    //
    // Initialized only once for the entire application lifetime
    //
    let pipeline_context = pipeline::Context::new(library);

    Box::into_raw(pipeline_context) as jlong
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_mdnssknght_mycamera_processing_NativeRawProcessor_00024Companion_nativeFini(
    _: JNIEnv,
    _: JClass,
    handle: jlong,
) {
    drop(unsafe { Box::from_raw(handle as *mut pipeline::Context) });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_mdnssknght_mycamera_processing_NativeRawProcessor_00024Companion_nativeProcess(
    env: JNIEnv,
    _: JClass,
    handle: jlong,
    width: jint,
    height: jint,
    data: JByteBuffer,
    out: JByteArray,
    color_filter_arrangement: jint,
    white_level: jint,
    black_level: JIntArray,
    neutral_point: JFloatArray,
    color_gains: JFloatArray,
    color_correction_transform: JFloatArray,
    forward_matrix_1: JFloatArray,
    forward_matrix_2: JFloatArray,
) {
    let context = unsafe { &*(handle as *const pipeline::Context) };

    let black_level = {
        let mut data = [0i32; 4];
        env.get_int_array_region(black_level, 0, &mut data).unwrap();
        data
    };

    let neutral_point = {
        let mut data = [0f32; 3];
        env.get_float_array_region(neutral_point, 0, &mut data)
            .unwrap();
        data
    };

    let color_gains = {
        let mut data = [0f32; 4];
        env.get_float_array_region(color_gains, 0, &mut data)
            .unwrap();
        data
    };

    let color_correction_transform = {
        let mut data = [0f32; 9];
        env.get_float_array_region(color_correction_transform, 0, &mut data)
            .unwrap();
        data
    };

    let forward_matrix_1 = {
        let mut data = [0f32; 9];
        env.get_float_array_region(forward_matrix_1, 0, &mut data)
            .unwrap();
        data
    };

    let forward_matrix_2 = {
        let mut data = [0f32; 9];
        env.get_float_array_region(forward_matrix_2, 0, &mut data)
            .unwrap();
        data
    };

    let mut finish = pipeline::Finish::new();

    finish.finish(
        &context,
        env.get_direct_buffer_address(&data).unwrap(),
        env.get_direct_buffer_capacity(&data).unwrap(),
        [width, height],
        color_filter_arrangement,
        white_level,
        black_level,
        neutral_point,
        color_gains,
        color_correction_transform,
        forward_matrix_1,
        forward_matrix_2,
    );

    let output_buffer = match finish.get_buffer_output() {
        Some(buffer_guard) => {
            let buffer = buffer_guard
                .read()
                .expect("Failed to lock buffer for reading");
            unsafe { slice::from_raw_parts(buffer.as_ptr() as *const jbyte, buffer.len()) }
        }
        _ => panic!("Something went wrong"),
    };

    env.set_byte_array_region(out, 0, output_buffer).unwrap();

    info!("Command buffer execution succeeded");
}
