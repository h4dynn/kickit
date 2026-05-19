# Installing

## Void Linux

A package is available for Void Linux. Keep in mind this **WILL replace runit**, so be very
careful to make sure you know what you're doing!

You can add the repository to have `kickit` update alongside your other packages:

```sh
$ echo "repository=https://h4dynn.github.io/void-linux" | sudo tee /etc/xbps.d/90-kickit-repository.conf
```

And then, install it:

```sh
$ sudo xbps-install -S kickit
```

## Other distros

Packages for other distros will be added in the future!
