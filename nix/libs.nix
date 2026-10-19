# Libraries winit/Slint load at runtime (dlopen). NixOS does not expose them
# globally, so the package wraps the binary with these on LD_LIBRARY_PATH.
{ wayland, libxkbcommon, libGL, vulkan-loader, fontconfig, freetype, libx11, libxcursor, libxi, libxrandr, libxcb }:
[ wayland libxkbcommon libGL vulkan-loader fontconfig freetype libx11 libxcursor libxi libxrandr libxcb ]
