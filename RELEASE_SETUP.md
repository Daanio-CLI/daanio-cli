# Release setup

The source tree is fully renamed and the documented Daanio API profile points
to `https://api.daanio.com/v1`. Before publishing binaries:

1. Publish this repository under the intended GitHub owner.
2. Set `DAANIO_GITHUB_REPOSITORY=owner/repository` for update checks and release
   automation.
3. Set `DAANIO_REPO_URL` to the fork's clone URL for self-development cloning.
   `/support` defaults to `support@daanio.com`; set `DAANIO_SUPPORT_EMAIL` only
   for a distribution with a different support address.
4. Configure installer release metadata or distribute from GitHub Releases.
5. Configure `DAANIO_TELEMETRY_ENDPOINT` only if you operate a compliant
   telemetry service and have obtained appropriate user consent. Telemetry is
   disabled when this variable is absent.
6. Configure `[sponsors] endpoint` only if you operate or trust a discovery
   directory. Sponsored discovery is disabled by default.
7. Keep the undocumented legacy managed-subscription/device-authorization path
   disabled unless its server contract and URLs are separately implemented and
   verified. The public `daanio` provider uses the documented API-key flow.
8. Replace or remove the inherited demo artwork and verify trademark rights for
   all public branding assets.
9. Generate and bundle third-party license notices for the exact release
   feature set. The current declared-license scan is predominantly MIT and
   Apache-2.0; `option-ext` is MPL-2.0, while `self_cell` and `r-efi` offer
   permissive Apache/MIT alternatives. Review the actual release artifact with
   legal counsel or a tool such as `cargo-about` before commercial distribution.

Never point Daanio release automation at the upstream jcode repository: an
upstream binary would replace the fork's executable and configuration identity.
