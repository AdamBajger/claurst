# GPG SETUP FOR DEVCONTAINER SIGNING (HOST MACHINE + CONTAINER)

GOAL:
Enable git commit signing inside a Docker devcontainer using host GPG keys via agent forwarding.


## 1. VERIFY GPG WORKS ON HOST


```bash
gpg --list-secret-keys
```
You should see your GPG keys listed.
```text
/home/johndoe/.gnupg/pubring.kbx
-----------------------------------
sec   ed25519 2001-09-11 [SC]
      ABCDEFGHIJKLMNOPQRSTUVWXYZ123456789101112
uid           [ unknown] John Doe <john.doe@example.com>
uid           [ unknown] John Doe <jd@some.domain.net>

ssb   cv25519 2001-09-11 [E]
```
Next try signing a test message:

```bash
echo "test" | gpg --clearsign
```
You should see a signed message without errors. The terminal may prompt you for your GPG key passphrase if it is not cached by the agent. If you see TTY errors, skip to step 3 to configure pinentry.
```text
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA512

test
-----BEGIN PGP SIGNATURE-----

4XI5AQSpeedMFnOZZleCawhLeRcuj8WH5O3/9PSw2hjf1UrlHwEAml+bcoXy/Q90
iHUEAREKAB00MQTVcSWhyAreYouGehhSEXisAWESOMEafys8AK47isTheBestwA1
ChillManvGGQbtzF6103rGfHM1jHkQ4=
=hklc
-----END PGP SIGNATURE-----
```
If this works, continue.


## 2. ENSURE GPG AGENT IS RUNNING


```bash
gpgconf --launch gpg-agent
```

Check socket location:

```bash
gpgconf --list-dirs agent-socket
```

Typical output:
```text
/run/user/1000/gnupg/S.gpg-agent
```

## 3. CONFIGURE PINENTRY (IMPORTANT)


Edit or create file `~/.gnupg/gpg-agent.conf`
Add the line `allow-loopback-pinentry`

Then, add a line setting the pinentry program. If you want the VS Code git extension to work with GPG keys that have passphrases, you must use a graphical pinentry program. For example, on Debian/Ubuntu you can use `pinentry-gtk2`:

```text
allow-loopback-pinentry
pinentry-program /usr/bin/pinentry-gtk-2
```

> TIP 💡: </br> If you want to use the terminal pinentry, you can set it to `pinentry-curses` or `pinentry-tty`, but this may cause TTY errors in some environments and you may need to ensure `GPG_TTY` is set correctly (see step 4). Also, you must commit using the terminal to see the passphrase prompt.

Edit file `~/.gnupg/gpg.conf`
Add the lines 
```text
pinentry-mode loopback
use-agent
```

Restart agent:
```bash
gpgconf --kill gpg-agent
gpgconf --launch gpg-agent
```


## 4. OPTIONAL BUT RECOMMENDED (AVOID TTY ISSUES)


Ensure environment variable `GPG_TTY` exists on host shell.

Add `export GPG_TTY=$(tty)` to your shell config `~/.bashrc` or `~/.zshrc`. This ensures GPG can find the correct terminal for pinentry when using loopback mode. It is especially important if you use terminal-based pinentry programs. If you use a graphical pinentry, this is probably not necessary.

```bash
echo "test" | gpg --clearsign --batch --yes
```

OR without batch (if you want to test passphrase prompt):

```bash
echo "test" | gpg --clearsign
```
You should NOT get TTY errors.


## 5. DEVCONTAINER REQUIREMENT (HOST SIDE PREP)


Ensure agent socket exists and is accessible:

```bash
gpgconf --list-dirs agent-socket
```
Make note of the socket path (e.g. `/run/user/1000/gnupg/S.gpg-agent`). You will need this for the devcontainer configuration. Alternatively, you can use the `GPG_AGENT_SOCK` environment variable to specify the socket path when running the container. Add the following line to your shell config `~/.bashrc` or `~/.zshrc` to set it automatically:

```bash
export GPG_AGENT_SOCK=$(gpgconf --list-dirs agent-socket)
```

This is what will be mounted into container.
