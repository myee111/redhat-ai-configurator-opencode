%global debug_package %{nil}

Name:           redhat-ai-configurator
Version:        0.1.3
Release:        1%{?dist}
Summary:        Guided installer that configures goose for Red Hat AI MaaS and EnMaaS

License:        MIT
URL:            https://github.com/myee111/redhat-ai-configurator
Source0:        %{name}-%{version}.tar.gz
Source1:        %{name}-%{version}-vendor.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  make

# The RHEL extensions package provides the goose CLI this tool configures.
# Enable the repository first:
#   subscription-manager repos --enable=rhel-10-for-$(arch)-extensions-rpms
Requires:       goose >= 1.38
# Optional: lets the user store keys in the system keyring instead of a file.
Recommends:     libsecret-tools

%description
redhat-ai-configurator is a terminal installer for the goose AI agent. It
guides you through creating Red Hat AI MaaS (rhai-maas) and EnMaaS API keys,
validates them, and writes goose's configuration without touching unrelated
settings.

Service endpoints are configured, not embedded. Set them in
/etc/redhat-ai-configurator/endpoints.conf or through environment variables;
see the example file shipped with this package.

Keys are stored in ~/.config/goose/secrets.yaml with mode 0600, or in the
system keyring when you choose that option. The tool never creates keys and
never reads them back out of goose.

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
* Thu Oct 29 2026 Matthew Yee <yee.matthew@gmail.com> - 0.1.3-1
- Fix key validation requesting /v1/models/models instead of /v1/models

* Thu Oct 29 2026 Matthew Yee <yee.matthew@gmail.com> - 0.1.2-1
- Treat HTTP 5xx during key validation as a service error, show the response
  body, and allow the user to continue

* Thu Oct 29 2026 Matthew Yee <yee.matthew@gmail.com> - 0.1.1-1
- Check placeholder warnings and doctor results only for configured providers

* Thu Oct 29 2026 Matthew Yee <yee.matthew@gmail.com> - 0.1.0-1
- Initial package
