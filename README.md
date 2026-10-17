# AresClient

Base technique en Rust pour une DLL Windows x64 chargée dans un processus Java 21.
Elle se connecte à la JVM existante, ouvre une connexion JVMTI, écrit des logs et
se décharge avec **END**.

## Compiler

Utiliser la chaîne Rust `x86_64-pc-windows-msvc` et les outils de compilation C++
de Visual Studio. Cargo télécharge les dépendances depuis crates.io et les garde
dans son cache utilisateur. Le premier build nécessite un accès réseau si elles
ne sont pas déjà en cache. `Cargo.lock` fixe les versions utilisées et doit être
conservé dans Git avec `Cargo.toml`.

```powershell
cargo build --release
```

La DLL est produite dans `target/release/ares_client.dll`.

## Utiliser

1. Démarrer le client Java avec un JDK 21 x64.
2. Charger la DLL dans son processus Java, par exemple avec Process Hacker.
   Le chargement doit passer par le chargeur Windows normal, comme `LoadLibrary`.
   Le chargement manuel de l'image en mémoire (manual map) n'est pas pris en charge.
3. Vérifier le message de connexion dans `%TEMP%\ares_client-<PID>.log`.
   Les mêmes messages sont envoyés au débogueur Windows (`OutputDebugString`).
4. Appuyer sur **END** pour nettoyer JVMTI, détacher le thread JNI si nécessaire,
   fermer le journal et décharger la DLL.

Si le nettoyage échoue, la DLL reste chargée et l'erreur est journalisée.
Relâcher END et appuyer de nouveau pour réessayer. Une touche maintenue ne lance
pas plusieurs tentatives. Une erreur de connexion initiale laisse également END
disponible pour décharger la DLL.

## Organisation et cycle de vie

- `src/lib.rs` : entrée Windows minimale et thread du client.
- `src/jvm.rs` : découverte de la JVM et propriété de la connexion JVMTI.
- `src/hotkey.rs` : surveillance de END et nouvelles tentatives.
- `src/log.rs` : fichier par processus et sortie débogueur.

Aucune console n'est créée dans le processus Java. Pour suivre le journal depuis
un terminal séparé, remplacer `1234` par le PID du processus :

```powershell
Get-Content "$env:TEMP\ares_client-1234.log" -Wait
```

Le thread du client possède son objet `Jvm`. Aucun `JNIEnv` ni référence locale
ne sort d'une opération. Un attachement JNI créé par le client est de type daemon
et est détaché à la fin de l'opération, même en cas d'erreur. Un attachement existant
est emprunté sans être détaché. Le client ne détruit jamais la JVM de l'application.

Cette base ne demande aucune capacité JVMTI supplémentaire, n'installe aucun
callback et ne transforme aucune classe. Elle n'a donc ni relais assembleur ni
allocation de code persistante après le déchargement. Ajouter des callbacks
nécessiterait de revoir leur arrêt avant de décharger la DLL.

L'environnement JVMTI d'origine reste conservé tant que sa fermeture n'a pas
réussi. Les destructeurs Rust sont exécutés avant `FreeLibraryAndExitThread`, qui
ne déroule pas la pile Rust. Aucun travail JNI/JVMTI n'est effectué dans `DllMain`.

## Vérifier

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --release
```

Les tests unitaires couvrent les erreurs d'initialisation, la propriété de
l'attachement JNI, la conservation du bon environnement JVMTI, le nettoyage
répété et le réarmement de END après un échec.

Un test supplémentaire démarre une vraie JVM, avec les contrôles JNI activés,
et vérifie trois cycles de connexion et de nettoyage :

```powershell
$env:JAVA_HOME = 'C:\lib\java JDKs\jdk-21.0.12.1'
$env:PATH = "$env:JAVA_HOME\bin;$env:PATH"
cargo test jvm::smoke::jdk21_lifecycle -- --ignored --exact --nocapture --test-threads=1
```

Ce test ne synthétise aucune touche et ne s'injecte dans aucun autre processus.
La vérification finale dans l'application consiste à charger la DLL, appuyer sur
END, vérifier que le processus reste utilisable, puis charger la DLL à nouveau.
