# Vendetta Chess — macOS: "app is damaged" / "app endommagée" / "app está dañada"

---

## English

### The problem

After downloading `Vendetta Chess.dmg` from GitHub and opening it, you can
mount the disk image without issue, but when you try to open
**Vendetta Chess.app**, macOS shows:

> "Vendetta Chess" is damaged and can't be opened. You should move it to
> the Trash.

**This does not mean the file is actually corrupted.** It is macOS
Gatekeeper's (misleading) way of refusing to run an app that isn't signed
with a paid Apple Developer ID certificate and notarized by Apple. Any app
downloaded from the internet gets a hidden "quarantine" flag attached by
your browser; combined with the lack of a Developer ID signature, recent
versions of macOS show this "damaged" message instead of the older,
more honest "unidentified developer" warning.

Vendetta Chess is free, open-source software distributed without a paid
Apple Developer ID ($99/year) — this is the only reason you see this
message. The app itself is not corrupted.

### How to confirm this is the cause

Open **Terminal** (Applications → Utilities → Terminal) and run:

```bash
xattr -p com.apple.quarantine "/Applications/Vendetta Chess.app"
```

If this prints something (instead of an error like "No such xattr"), the
quarantine flag is indeed present, confirming the diagnosis above.

### The fix

In the same Terminal window, run:

```bash
xattr -cr "/Applications/Vendetta Chess.app"
```

(Adjust the path if you placed the app somewhere other than
`/Applications`.) This removes the quarantine flag. Vendetta Chess will
now open normally — a simple right-click → Open is **not** enough for
this particular "damaged" message; the Terminal command above is
required.

You only need to do this once per download.

---

## Français

### Le problème

Après avoir téléchargé `Vendetta Chess.dmg` depuis GitHub et l'avoir
ouvert, vous pouvez monter l'image disque sans problème, mais lorsque
vous essayez d'ouvrir **Vendetta Chess.app**, macOS affiche :

> « Vendetta Chess » est endommagé et ne peut pas être ouvert. Vous
> devriez le mettre à la corbeille.

**Cela ne signifie pas que le fichier est réellement corrompu.** C'est la
façon (trompeuse) qu'a Gatekeeper, le système de protection de macOS, de
refuser d'exécuter une application qui n'est pas signée avec un
certificat Apple Developer ID payant et notariée par Apple. Toute
application téléchargée depuis internet reçoit un attribut caché de
« quarantaine » posé par le navigateur ; combiné à l'absence de signature
Developer ID, les versions récentes de macOS affichent ce message
« endommagé » au lieu de l'ancien avertissement, plus honnête,
« développeur non identifié ».

Vendetta Chess est un logiciel libre et gratuit, distribué sans
certificat Apple Developer ID payant (99 $/an) — c'est l'unique raison de
ce message. L'application elle-même n'est pas corrompue.

### Comment confirmer que c'est bien la cause

Ouvrez **Terminal** (Applications → Utilitaires → Terminal) et exécutez :

```bash
xattr -p com.apple.quarantine "/Applications/Vendetta Chess.app"
```

Si une valeur s'affiche (au lieu d'une erreur du type « No such xattr »),
l'attribut de quarantaine est bien présent, ce qui confirme le
diagnostic ci-dessus.

### La solution

Dans la même fenêtre Terminal, exécutez :

```bash
xattr -cr "/Applications/Vendetta Chess.app"
```

(Adaptez le chemin si vous avez placé l'application ailleurs que dans
`/Applications`.) Cela retire l'attribut de quarantaine. Vendetta Chess
s'ouvrira alors normalement — un simple clic droit → Ouvrir **ne suffit
pas** pour ce message précis d'application « endommagée » ; la commande
Terminal ci-dessus est nécessaire.

Cette manipulation n'est à faire qu'une seule fois par téléchargement.

---

## Español

### El problema

Después de descargar `Vendetta Chess.dmg` desde GitHub y abrirlo, puede
montar la imagen de disco sin problema, pero al intentar abrir
**Vendetta Chess.app**, macOS muestra:

> «Vendetta Chess» está dañado y no se puede abrir. Debería moverlo a la
> papelera.

**Esto no significa que el archivo esté realmente corrupto.** Es la
forma (engañosa) en que Gatekeeper, el sistema de protección de macOS,
se niega a ejecutar una aplicación que no está firmada con un
certificado Apple Developer ID de pago y notarizada por Apple. Cualquier
aplicación descargada de internet recibe un atributo oculto de
«cuarentena» añadido por el navegador; combinado con la falta de firma
Developer ID, las versiones recientes de macOS muestran este mensaje de
«dañado» en lugar de la advertencia anterior, más honesta, de
«desarrollador no identificado».

Vendetta Chess es un software libre y gratuito, distribuido sin
certificado Apple Developer ID de pago (99 $/año) — esta es la única
razón de este mensaje. La aplicación en sí no está corrupta.

### Cómo confirmar que esta es la causa

Abra **Terminal** (Aplicaciones → Utilidades → Terminal) y ejecute:

```bash
xattr -p com.apple.quarantine "/Applications/Vendetta Chess.app"
```

Si esto imprime algún valor (en lugar de un error como «No such xattr»),
el atributo de cuarentena está efectivamente presente, lo que confirma
el diagnóstico anterior.

### La solución

En la misma ventana de Terminal, ejecute:

```bash
xattr -cr "/Applications/Vendetta Chess.app"
```

(Ajuste la ruta si colocó la aplicación en un lugar distinto de
`/Applications`.) Esto elimina el atributo de cuarentena. Vendetta Chess
se abrirá entonces con normalidad — un simple clic derecho → Abrir **no
es suficiente** para este mensaje concreto de aplicación «dañada»; el
comando de Terminal anterior es necesario.

Solo es necesario hacer esto una vez por descarga.
