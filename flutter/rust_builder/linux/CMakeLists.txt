cmake_minimum_required(VERSION 3.10)

set(PROJECT_NAME "shosai_flutter_bridge")
project(${PROJECT_NAME} LANGUAGES CXX)

find_program(CARGO_EXECUTABLE cargo REQUIRED)
get_filename_component(PLUGIN_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/.." REALPATH)
get_filename_component(WORKSPACE_ROOT "${PLUGIN_ROOT}/../.." REALPATH)
set(RUST_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo-target")

if(CMAKE_BUILD_TYPE STREQUAL "Debug")
  set(CARGO_PROFILE "debug")
  set(CARGO_PROFILE_FLAG "")
else()
  set(CARGO_PROFILE "release")
  set(CARGO_PROFILE_FLAG "--release")
endif()

set(RUST_LIBRARY
  "${RUST_TARGET_DIR}/${CARGO_PROFILE}/libshosai_flutter_bridge.so"
)

add_custom_target(${PROJECT_NAME}_cargo ALL
  COMMAND "${CMAKE_COMMAND}" -E env
    "CARGO_TARGET_DIR=${RUST_TARGET_DIR}"
    "${CARGO_EXECUTABLE}" build
    --manifest-path "${WORKSPACE_ROOT}/Cargo.toml"
    --package shosai-flutter-bridge
    ${CARGO_PROFILE_FLAG}
  BYPRODUCTS "${RUST_LIBRARY}"
  WORKING_DIRECTORY "${WORKSPACE_ROOT}"
  VERBATIM
)

if(NOT PDFIUM_LIBRARY)
  string(REPLACE ":" ";" PDFIUM_HINTS "$ENV{LD_LIBRARY_PATH}")
  find_library(PDFIUM_LIBRARY pdfium HINTS ${PDFIUM_HINTS} REQUIRED)
endif()

set(shosai_flutter_bridge_bundled_libraries
  "${RUST_LIBRARY}"
  "${PDFIUM_LIBRARY}"
  PARENT_SCOPE
)
