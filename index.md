---
pagetitle: Fediverse nodes list
---

<h1>&nbsp;<!-- spacer --></h1>

<p style="text-align: center;">
👉 <a href="./nodes.json">nodes.json</a> 👈
</p>

<h1>&nbsp;<!-- spacer --></h1>

## Why does this exist?

To **help keep track of Fediverse's growth** (or stagnation T_T).

[The-federation.info][the-federation], [FediDB.org][fedidb],
[Fediverse.Observer][fediverse.observer] and other public hubs provide some
numbers, but they don't automatically discover new servers. As known instances
slowly disappear, these hubs can give an impression that Fediverse is shrinking.
This crawler discovers new nodes so hubs don't have to.

[the-federation]: https://the-federation.info "the federation — a statistics hub"
[fedidb]: https://fedidb.org "FediDB — Developer Tools for ActivityPub"
[fediverse.observer]: https://fediverse.observer "Fediverse Observer"

Another *raison d'être* is to **enable novel applications** that need such
a list. We don't know what they are just yet. A service that recommends
instances to newcomers? Some sort of a cataloguing effort? Global search? We
want you to go straight to building *that*, without spending your energy on
re-inventing the "Fediverse crawler" wheel.

## How to remove a node from this list?

You can only do this if you're the node's administrator.

If you're running **GNU Social** or **Friendica**: set the `site.private`
property to `1` or `true` in the StatusNet config.

If you're running **Hubzilla**: set `hide_in_statistics` property to `1` or
`true` in `siteinfo.json`.

If you're running **anything else**: add the following to your `robots.txt`:

```
User-agent: MinoruFediverseCrawler
Disallow: /
```

## How to add a node to this list?

Simply follow someone on Fediverse ツ Soon enough, the crawler will discover the
node and add it to the list.

Make sure the instance didn't opt out of statistics as described above.

If you did that, and after a week the instance still isn't on the list, please
<a href="https://github.com/Minoru/minoru-fediverse-crawler/issues/new">file an
issue</a>.

## What exactly does the list contain?

It's a JSON array of hostnames of all alive nodes that are known to the crawler.

"Alive" means any instance that responded with NodeInfo at least once within the
last week. It doesn't imply that the instance federates with anyone, or has a web
UI, or is working *right now*.

Conversely, if an instance is missing from this list, it doesn't mean the
instance doesn't exist. It could be blocking access to NodeInfo, or its address
could be unreachable from the crawler's host (as is the case with Tor and I2P
addresses).

"Known" means that the crawler has seen the hostname in someone's peer list. As
of 2021-11-09, the crawler requests `/api/v1/peers` endpoint from Mastodon,
Pleroma, Misskey, BookWyrm, and Smithereen servers. If an instance doesn't
federate with anyone, it would be missing from the peers lists, and the crawler
won't know about its existence.

Only hostnames are included in the list; no ports, no URL schemas (HTTPS and 443
are assumed). Furthermore, only hostnames whose suffixes are on the [Public
Suffix List][publicsuffix] are allowed.

[publicsuffix]: https://publicsuffix.org/ "Public Suffix List"

## How often is the list updated?

About every six hours.

Please do not fetch the list very often. It doesn't make sense; only a couple
instances appear and die every day, and you probably don't need to know about it
*right away*. This is not a monitoring service.

The nodes themselves are checked about once a day. The crawler also maintains
internal lists of "moved" and "dead" instances, which are checked once a week
(just in case they come back to life). The checks are spread throughout the day
with some jitter, that's why the list is updated more often than the check
period.

Crawler's user agent string is `Minoru's Fediverse Crawler
(+https://nodes.fediverse.party)`, and it makes requests from the following IP
addresses:

* 84.22.103.136
* 2a02:2770:8:0:21a:4aff:fefd:4598

## Who is responsible for this list?

Alexander Batischev AKA Minoru, whom you can reach on Fediverse at
[\@minoru@functional.cafe][minoru] or by email at <a
href="mailto:eual.jp@gmail.com">eual.jp@gmail.com</a>. My PGP key is
0x356961a20c8bfd03.

Kudos to [\@lightone@mastodon.xyz][lightone] for all the discussions and all the
ideas she brought to this project!

[minoru]: https://functional.cafe/@minoru "Minoru (@minoru@functional.cafe)"
[lightone]: https://mastodon.xyz/@lightone "lostinlight (@lightone@mastodon.xyz)"

## Where is the code?

See
[github.com/Minoru/minoru-fediverse-crawler](https://github.com/Minoru/minoru-fediverse-crawler).
I'll gladly move to a self-hosted Gitea instance once ForgeFed becomes a reality ^_^

The code is licensed under AGPL 3.0+.
