#pragma once

#include <cstddef>
#include <cstdint>

extern "C" {
// Reference-counting helpers shared by every generated translation unit.
void rvtk_register(void* obj);
void rvtk_delete(void* obj);

// Heap string helpers for `std::string` returns.
char* rvtk_strdup(const char* s);
void rvtk_free(void* p);
}
