# Putting the Workbench on a server of your own

Started at an address, the Workbench is reached from elsewhere and whoever comes signs in with a
passkey. It needs a name and TLS: a certificate of its own, or your proxy in front.

```text
swem workbench serve --at https://workbench.example.org --tls-cert chain.pem --tls-key key.pem
swem workbench serve --at https://workbench.example.org --behind-proxy \
     --apps-at https://apps.workbench.example.org
```

The first start prints a word that is used once. Open the address, give it the word and register
your device; you are shown codes to come back with, once. A second device is added from one that
is signed in, under Settings, Access, where tokens for programs are made as well. Anything that
would leave the way open - an address without TLS, a listener beyond this machine with no
certificate - is refused in words. A certificate renewed in its files is taken up without a
restart. Without a name, keep it on the server's own loopback and reach it through an SSH tunnel.

Building needs the system's OpenSSL headers (`libssl-dev`, `openssl-devel`, or Homebrew's
`openssl@3`), which the passkey library links.
