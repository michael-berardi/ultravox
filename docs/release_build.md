## Release build

```shell
NOTARYTOOL_PROFILE="<your-notarytool-keychain-profile>" ./notarize_app.sh "$CODE_SIGN_IDENTITY"
```

Example:
```shell
NOTARYTOOL_PROFILE="MyNotaryProfile" ./notarize_app.sh "Developer ID Application: AAAA BBBB (XXXXX)" 
```


