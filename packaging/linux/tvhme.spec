# Binary package built from an already compiled program (see .github/workflows/linux.yml).
# Build:  rpmbuild -bb --define "_topdir $PWD/rpmbuild" --define "srcroot $PWD" --define "pkgversion 0.1.0" packaging/linux/tvhme.spec
%global debug_package %{nil}
%global __strip /bin/true
%global _build_id_links none

Name:           tvhme
Version:        %{pkgversion}
Release:        1%{?dist}
Summary:        Native Tvheadend client
License:        LicenseRef-unspecified
URL:            https://github.com/mmBesar/TVHme
Recommends:     mpv
Requires:       fontconfig, libxkbcommon

%description
Watch live TV and recordings from a Tvheadend server, browse the programme guide,
and schedule recordings. Playback uses mpv (or VLC).

%prep

%build

%install
install -Dm755 %{srcroot}/target/release/tvhme %{buildroot}%{_bindir}/tvhme
install -Dm644 %{srcroot}/packaging/linux/tvhme.desktop %{buildroot}%{_datadir}/applications/tvhme.desktop
sed -i 's|^Exec=.*|Exec=tvhme|' %{buildroot}%{_datadir}/applications/tvhme.desktop
install -Dm644 %{srcroot}/assets/icon.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/tvhme.svg
for s in 16 24 32 48 64 128 256 512; do
  install -Dm644 %{srcroot}/assets/icons/${s}x${s}.png %{buildroot}%{_datadir}/icons/hicolor/${s}x${s}/apps/tvhme.png
done

%files
%{_bindir}/tvhme
%{_datadir}/applications/tvhme.desktop
%{_datadir}/icons/hicolor/*/apps/tvhme.*

%changelog
* Fri Oct 23 2026 mmbesar <mbesar@proton.me> - %{pkgversion}-1
- Packaged build
