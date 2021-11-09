---
pagetitle: Fediverse instances list
---

<center>
<h1>Fediverse instances list</h1>
</center>

[Fediverse][wikipedia-fediverse] has thousands of servers. It's hard to get an
accurate measure of how many there are, and how many people are using them. To
help with that, we compile a list of all known alive instances, and publish it
here:

<center>
<a href="./instances.json">instances.json</a>
</center>

<div style="height: 4rem;"></div>

[wikipedia-fediverse]: https://en.wikipedia.org/wiki/Fediverse "Fediverse — Wikipedia"

## How to remove an instance from this list?

You can only do this if you're the instance's administrator.

If you're running **GNU Social** or **Friendica**: set the `site.private`
property to `1` or `true` in the StatusNet config.

If you're running **Hubzilla**: set `hide_in_statistics` property to `1` or
`true` in `siteinfo.json`.

If you're running anything else: add the following to your `robots.txt`:

```
User-agent: MinoruFediverseCrawler
Disallow: /
```

## How to add an instance to this list?

Simply follow someone on the Fediverse :) Soon enough, the crawler will discover
the instance and add it to the list.

Make sure the instance didn't opt out of statistics as described above.

## What exactly does the list contain?

It's a JSON array of hostnames of all alive instances that are known to the
crawler.

"Alive" means any instance that responded with NodeInfo at least once within the
last week. It doesn't imply that the instance federates with anyone, or has a web
UI, or is working *right now*.

Conversely, if an instance is missing from this list, it doesn't mean the
instance doesn't exist. It could be blocking access to NodeInfo, or its address
could be unreachable from the crawler's host (as is the case with Tor and I2P
addresses).

"Known" means that the crawler have seen the hostname in someone's peer list. As
of 2021-11-09, the crawler requests `/api/v1/peers` endpoint from Mastodon,
Pleroma, Misskey, BookWyrm, and Smithereen servers. If an instance doesn't
federate with anyone, it would be missing from the peers lists, and the crawler
won't know about its existence.

Only hostnames are included in the list; no ports, no URL schemas (HTTPS and 443
are assumed). Furthermore, only hostnames whose suffixes are on the [Public
Suffix List][publicsuffix] are allowed.

[publicsuffix]: https://publicsuffix.org/ "Public Suffix List"

## How often is the list updated?

About every half an hour.

The instances themselves are checked about once a day. The crawler also
maintains internal lists of "moved" and "dead" instances, which are checked once
a week (just in case any of them come back to life). The checks are spread out
throughout the day with some jitter, that's why the list is updated more often
than the check period.

## Who is responsible for this list?

Alexander Batischev AKA Minoru, whom you can reach on Fediverse at <a href="https://functional.cafe/@minoru">@minoru@functional.cafe</a> or by email at <a href="mailto:eual.jp@gmail.com">eual.jp@gmail.com</a>. My PGP key is 0x356961a20c8bfd03.

## Where is the code?

See
[github.com/Minoru/minoru-fediverse-crawler](https://github.com/Minoru/minoru-fediverse-crawler).
I'll gladly move to a self-hosted Gitea instance once ForgeFed becomes a reality :)

The code is licensed under AGPL 3.0+.
