cmake_minimum_required(VERSION 3.10)

project(kanari_crypto_plugin LANGUAGES CXX)

add_library(kanari_crypto_plugin SHARED
  "kanari_crypto_plugin.cpp"
  "include/kanari_crypto/kanari_crypto_plugin.h"
)

apply_standard_settings(kanari_crypto_plugin)

target_include_directories(kanari_crypto_plugin INTERFACE
  "${CMAKE_CURRENT_SOURCE_DIR}/include"
)

target_compile_definitions(kanari_crypto_plugin PUBLIC
  "FLUTTER_PLUGIN_SYMBOL=KANARI_CRYPTO_PLUGIN_EXPORT"
)

target_link_libraries(kanari_crypto_plugin PRIVATE flutter)
target_link_libraries(kanari_crypto_plugin PRIVATE flutter_wrapper_plugin)

# Copy the Rust FFI DLL to the output directory.
set(KANARI_CRYPTO_RUST_DLL "${CMAKE_CURRENT_SOURCE_DIR}/../lib/rust.dll")

add_custom_command(
  TARGET kanari_crypto_plugin POST_BUILD
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
    "${KANARI_CRYPTO_RUST_DLL}"
    "$<TARGET_FILE_DIR:kanari_crypto_plugin>/rust.dll"
  COMMENT "Copying Kanari Rust FFI library"
)
