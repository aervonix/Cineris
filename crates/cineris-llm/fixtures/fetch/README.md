# The fetch client's TLS fixture

`leaf.der` is a certificate for `localhost` and `127.0.0.1`, and `leaf-key.der` its key (PKCS #8, P-256). They are for tests only and protect nothing. The certificate's issuer is a CA that was thrown away when they were made, so nothing trusts it. It names three URLs on `127.0.0.1:65432`: the issuer's certificate (`caIssuers`), an OCSP responder and a revocation list. A test writes a listening port of five digits over `65432`, which breaks the issuer's signature and leaves the key's half unchanged, then has a TLS server present the certificate. A verifier that fetched what a certificate names would connect to that port: the fetch client's must not (`cineris-llm::it fetch::a_servers_certificate_sends_the_machine_nowhere`).

They were made with OpenSSL 3.5:

```
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -keyout ca.key -out ca.pem -days 36500 \
  -subj "/CN=Cineris fetch test CA, thrown away"
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -keyout leaf.key -out leaf.csr -subj "/CN=localhost"
openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -set_serial 0x2801 \
  -days 36500 -extfile leaf.ext -out leaf.pem
openssl x509 -in leaf.pem -outform DER -out leaf.der
openssl pkcs8 -topk8 -nocrypt -in leaf.key -outform DER -out leaf-key.der
```

with `leaf.ext`:

```
basicConstraints=critical,CA:FALSE
subjectAltName=DNS:localhost,IP:127.0.0.1
authorityInfoAccess=caIssuers;URI:http://127.0.0.1:65432/issuer.crt,OCSP;URI:http://127.0.0.1:65432/ocsp
crlDistributionPoints=URI:http://127.0.0.1:65432/revoked.crl
keyUsage=critical,digitalSignature
extendedKeyUsage=serverAuth
```
