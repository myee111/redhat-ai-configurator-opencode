%global debug_package %{nil}

Name:           redhat-ai-configurator-opencode
Version:        0.1.0
Release:        1%{?dist}
Summary:        Installer that installs and configures OpenCode for Red Hat AI MaaS and EnMaaS

License:        MIT
URL:            https://github.com/myee111/redhat-ai-configurator-opencode
Source0:        %{name}-%{version}.tar.gz
Source1:        %{name}-%{version}-vendor.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  make

%description
redhat-ai-configurator-opencode is a terminal installer for the OpenCode AI
coding agent. When the opencode CLI is missing it downloads the current Linux
release and installs it for the invoking user; then it guides you through
creating Red Hat AI MaaS (rhai-maas) and EnMaaS API keys, validates them, and
writes OpenCode's configuration without touching unrelated settings.

Service endpoints are configured, not embedded. Set them in
/etc/redhat-ai-configurator-opencode/endpoints.conf or through environment
variables; see the example file shipped with this package.

Keys are stored in OpenCode's own credential store through `opencode auth
import`. The tool never creates keys, never edits OpenCode's database directly,
and never prints a key back.

%prep
%autosetup -n %{name}-%{version}
tar -xzf %{SOURCE1}

%build
export CARGO_HOME="$PWD/.cargo-home"
export CARGO_NET_OFFLINE=true
cargo build --release --offline --locked

%install
install -Dm0755 target/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dm0644 packaging/%{name}.1 %{buildroot}%{_mandir}/man1/%{name}.1
install -Dm0644 packaging/endpoints.conf.example %{buildroot}%{_sysconfdir}/%{name}/endpoints.conf.example

%check
export CARGO_HOME="$PWD/.cargo-home"
export CARGO_NET_OFFLINE=true
cargo test --offline --locked

%files
%{_bindir}/%{name}
%{_mandir}/man1/%{name}.1*
%{_sysconfdir}/%{name}/endpoints.conf.example
%license LICENSE
%doc README.md

%changelog
* Wed Nov 04 2026 Matthew Yee <yee.matthew@gmail.com> - 0.1.0-1
- Initial package: installs the OpenCode CLI when missing and configures it for
  Red Hat AI MaaS and EnMaaS
