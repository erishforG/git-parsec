(function () {
  'use strict';

  var BASE = '/git-parsec';
  var MANIFEST = BASE + '/versions.json';

  function detectVersion() {
    var match = location.pathname.match(/\/git-parsec\/v\/([^/]+)\//);
    return match ? match[1] : 'latest';
  }

  function currentSubpath() {
    var path = location.pathname;
    // Strip base and optional version prefix
    var rest = path.replace(/^\/git-parsec\/?/, '').replace(/^v\/[^/]+\/?/, '');
    // rest is now '' or 'guide/' or 'reference/' etc.
    return rest;
  }

  function versionUrl(version, subpath) {
    if (version === 'latest') return BASE + '/' + subpath;
    return BASE + '/v/' + version + '/' + subpath;
  }

  function createDropdown(versions, current) {
    var container = document.createElement('div');
    container.className = 'version-switcher';

    var btn = document.createElement('button');
    btn.className = 'version-btn';
    btn.textContent = current === 'latest' ? 'latest' : 'v' + current;
    btn.setAttribute('aria-label', 'Switch documentation version');

    var arrow = document.createElement('span');
    arrow.className = 'version-arrow';
    arrow.textContent = ' \u25BE';
    btn.appendChild(arrow);

    var dropdown = document.createElement('div');
    dropdown.className = 'version-dropdown';

    // "latest" option
    var latestLink = document.createElement('a');
    latestLink.href = versionUrl('latest', currentSubpath());
    latestLink.textContent = 'latest (' + versions.latest + ')';
    if (current === 'latest') latestLink.className = 'active';
    dropdown.appendChild(latestLink);

    // Versioned options
    versions.versions.forEach(function (v) {
      var link = document.createElement('a');
      link.href = versionUrl(v.version, currentSubpath());
      link.textContent = 'v' + v.version;
      if (current === v.version) link.className = 'active';
      dropdown.appendChild(link);
    });

    container.appendChild(btn);
    container.appendChild(dropdown);

    btn.addEventListener('click', function (e) {
      e.stopPropagation();
      dropdown.classList.toggle('open');
    });

    document.addEventListener('click', function () {
      dropdown.classList.remove('open');
    });

    return container;
  }

  function showBanner(current, latest) {
    var banner = document.createElement('div');
    banner.className = 'version-banner';
    banner.innerHTML =
      'You are viewing docs for <strong>v' + current + '</strong>. ' +
      '<a href="' + versionUrl('latest', currentSubpath()) + '">View latest (' + latest + ') \u2192</a>';
    document.body.insertBefore(banner, document.body.firstChild);

    // Push nav and body content below the banner
    requestAnimationFrame(function () {
      var h = banner.offsetHeight;
      var nav = document.querySelector('nav');
      if (nav) nav.style.top = h + 'px';
      document.body.style.paddingTop = h + 'px';
    });
  }

  function init() {
    fetch(MANIFEST)
      .then(function (r) { return r.json(); })
      .then(function (data) {
        var current = detectVersion();
        var navBrand = document.querySelector('.nav-inner .nav-brand');
        if (!navBrand) return;

        var switcher = createDropdown(data, current);
        navBrand.parentNode.insertBefore(switcher, navBrand.nextSibling);

        if (current !== 'latest' && current !== data.latest) {
          showBanner(current, data.latest);
        }
      })
      .catch(function () {
        // Graceful degradation: no dropdown on fetch failure
      });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
