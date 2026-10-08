// Support routine for the link probe.
//
// The generated `CMakeLists.txt` builds `rvtk_link_probe`, a throw-away
// executable that links the static shim.  It exists so that
// `rvtk-sys/build.rs` can read CMake's own link line (and so that the static
// VTK archives are proven to link on this platform); it is never installed.
//
// The call to `rvtk_register` makes the linker pull the shim in, which in turn
// pulls in the static VTK archives.

extern "C" void rvtk_register(void* obj);

int main()
{
  rvtk_register(nullptr);
  return 0;
}
