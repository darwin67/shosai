cmake_minimum_required(VERSION 3.10)

set(PROJECT_NAME "shosai_flutter_bridge")
project(${PROJECT_NAME} LANGUAGES CXX)

find_program(CARGO_EXECUTABLE cargo REQUIRED)
get_filename_component(PLUGIN_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/.." REALPATH)
get_filename_component(WORKSPACE_ROOT "${PLUGIN_ROOT}/../.." REALPATH)
set(RUST_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo-target")
set(BUNDLE_DIR "${CMAKE_CURRENT_BINARY_DIR}/bundle")

if(CMAKE_BUILD_TYPE STREQUAL "Debug")
  set(CARGO_PROFILE "debug")
  set(CARGO_PROFILE_FLAG "")
else()
  set(CARGO_PROFILE "release")
  set(CARGO_PROFILE_FLAG "--release")
endif()

set(RUST_LIBRARY
  "${RUST_TARGET_DIR}/${CARGO_PROFILE}/libshosai_flutter_bridge.dylib"
)
set(BUNDLED_RUST_LIBRARY "${BUNDLE_DIR}/libshosai_flutter_bridge.dylib")

file(GLOB_RECURSE RUST_SOURCES CONFIGURE_DEPENDS
  "${WORKSPACE_ROOT}/crates/shosai-core/src/*.rs"
  "${WORKSPACE_ROOT}/crates/shosai-flutter-bridge/src/*.rs"
)

add_custom_command(
  OUTPUT "${BUNDLED_RUST_LIBRARY}"
  COMMAND "${CMAKE_COMMAND}" -E env
    "CARGO_TARGET_DIR=${RUST_TARGET_DIR}"
    "${CARGO_EXECUTABLE}" build
    --manifest-path "${WORKSPACE_ROOT}/Cargo.toml"
    --package shosai-flutter-bridge
    ${CARGO_PROFILE_FLAG}
  COMMAND "${CMAKE_COMMAND}" -E make_directory "${BUNDLE_DIR}"
  COMMAND "${CMAKE_COMMAND}" -E copy_if_different
    "${RUST_LIBRARY}" "${BUNDLED_RUST_LIBRARY}"
  DEPENDS
    ${RUST_SOURCES}
    "${WORKSPACE_ROOT}/Cargo.toml"
    "${WORKSPACE_ROOT}/Cargo.lock"
    "${WORKSPACE_ROOT}/crates/shosai-core/Cargo.toml"
    "${WORKSPACE_ROOT}/crates/shosai-flutter-bridge/Cargo.toml"
  WORKING_DIRECTORY "${WORKSPACE_ROOT}"
  VERBATIM
)

if(NOT PDFIUM_LIBRARY)
  string(REPLACE ":" ";" PDFIUM_HINTS "$ENV{DYLD_LIBRARY_PATH}")
  find_library(PDFIUM_LIBRARY pdfium HINTS ${PDFIUM_HINTS} REQUIRED)
endif()

set(BUNDLED_PDFIUM_LIBRARY "${BUNDLE_DIR}/libpdfium.dylib")
add_custom_command(
  OUTPUT "${BUNDLED_PDFIUM_LIBRARY}"
  COMMAND "${CMAKE_COMMAND}" -E make_directory "${BUNDLE_DIR}"
  COMMAND "${CMAKE_COMMAND}" -E copy_if_different
    "${PDFIUM_LIBRARY}" "${BUNDLED_PDFIUM_LIBRARY}"
  DEPENDS "${PDFIUM_LIBRARY}"
  VERBATIM
)

add_custom_target(${PROJECT_NAME}_bundle ALL DEPENDS
  "${BUNDLED_RUST_LIBRARY}"
  "${BUNDLED_PDFIUM_LIBRARY}"
)
