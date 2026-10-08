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

## Hosts of one person

Every `swem` has a key of its own and prints its name and fingerprint when it starts. A second
host - a server, a laptop, a machine that sleeps - is added from a page of one that is yours:
Settings, Hosts, Add a host, with the address the other shows and a word. A served host that
belongs to nobody yet offers the choice at its door: "This device is mine" (a passkey, as above)
or "Add it to my hosts", by the word it printed. Once added, its agents and chats stand in the
rail under its name, and opening one draws that host's own page through the host you are
signed in to; nothing is served twice. Hosts reach each other over iroh, by their keys, through
the relays of n0 unless `--relay <url>` names one of your own or `--relay none` keeps two hosts
on one machine direct. "Forget" on any page tells the host and takes it out of your hosts
everywhere.

Building needs the system's OpenSSL headers (`libssl-dev`, `openssl-devel`, or Homebrew's
`openssl@3`), which the passkey library links.
