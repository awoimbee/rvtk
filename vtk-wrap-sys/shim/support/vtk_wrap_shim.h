#pragma once

#include <cstddef>
#include <cstdint>

extern "C" {
// Reference-counting helpers shared by every generated translation unit.
void vtk_wrap_register(void* obj);
void vtk_wrap_delete(void* obj);

// Heap string helpers for `std::string` returns.
char* vtk_wrap_strdup(const char* s);
void vtk_wrap_free(void* p);
}
