// Скрипт страницы: тема и живое число звёзд.
//
// Он попадает в <head> телом, до разметки: тему надо решить раньше первой
// отрисовки, иначе тёмная страница мигнёт светлой.
(function () {
  "use strict";

  var KEY = "unica-theme";
  var root = document.documentElement;
  var dark = window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null;

  // Хранилище может быть недоступно: приватное окно, запрет на данные сайта.
  // Тема — украшение, поэтому отказ хранилища не должен ронять страницу.
  function stored() {
    try {
      var value = window.localStorage.getItem(KEY);
      return value === "light" || value === "dark" ? value : null;
    } catch (error) {
      return null;
    }
  }

  function remember(value) {
    try {
      window.localStorage.setItem(KEY, value);
    } catch (error) {
      /* не сохранилось — тема доживёт до конца страницы */
    }
  }

  function system() {
    return dark && dark.matches ? "dark" : "light";
  }

  function current() {
    return stored() || system();
  }

  function apply(theme) {
    root.setAttribute("data-theme", theme);
    var meta = document.querySelector('meta[name="theme-color"]');
    if (meta) {
      meta.setAttribute("content", theme === "dark" ? "#0D1117" : "#F8FAFC");
    }
    var button = document.querySelector(".theme");
    if (button) {
      // Значок показывает, куда переключит, а не где мы сейчас.
      var next = theme === "dark" ? "light" : "dark";
      button.setAttribute("data-shows", next);
      button.setAttribute("aria-label", next === "dark" ? "Тёмная тема" : "Светлая тема");
    }
  }

  apply(current());

  // Пока человек не выбрал сам, страница идёт за системой — в том числе
  // когда та переключается на закате, при уже открытой странице.
  if (dark && dark.addEventListener) {
    dark.addEventListener("change", function () {
      if (!stored()) {
        apply(system());
      }
    });
  }

  function ready(run) {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", run);
    } else {
      run();
    }
  }

  ready(function () {
    apply(current());

    var button = document.querySelector(".theme");
    if (button) {
      button.addEventListener("click", function () {
        var next = current() === "dark" ? "light" : "dark";
        remember(next);
        apply(next);
      });
    }

    // Блок, который прокручивается вбок, на телефоне об этом не сообщает:
    // полосы нет, а обрезанный край читается как конец содержимого. Поэтому,
    // впервые появившись на экране, он один раз качается и возвращается —
    // подсказка без единого слова. Тем, кто просил меньше движения, её не
    // показываем.
    var still = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)");
    var scrollable = document.querySelectorAll(".table-wrap, .addr");
    if (scrollable.length && window.IntersectionObserver && !(still && still.matches)) {
      // Кадры считаем сами: `scrollTo({behavior:"smooth"})` молча ничего не
      // делает там, где сглаживание отключено, и подсказка бы не показалась.
      var glide = function (node, from, to, span, done) {
        var start = 0;
        var step = function (now) {
          start = start || now;
          var part = Math.min(1, (now - start) / span);
          var eased = part < 0.5 ? 2 * part * part : 1 - Math.pow(-2 * part + 2, 2) / 2;
          node.scrollLeft = from + (to - from) * eased;
          if (part < 1) {
            window.requestAnimationFrame(step);
          } else if (done) {
            done();
          }
        };
        window.requestAnimationFrame(step);
      };

      var nudge = function (node) {
        var far = Math.min(30, node.scrollWidth - node.clientWidth);
        if (far < 10 || node.scrollLeft > 0) {
          return;
        }
        glide(node, 0, far, 280, function () {
          window.setTimeout(function () {
            glide(node, far, 0, 340);
          }, 140);
        });
      };
      var watcher = new IntersectionObserver(
        function (entries) {
          entries.forEach(function (entry) {
            if (entry.isIntersecting) {
              watcher.unobserve(entry.target);
              nudge(entry.target);
            }
          });
        },
        { threshold: 0.35 }
      );
      Array.prototype.forEach.call(scrollable, function (node) {
        watcher.observe(node);
      });
    }

    // Звёзды печатаются при сборке, поэтому число видно и без сети, и без
    // скрипта. Здесь оно только освежается: у публичного API нет ключа и
    // есть CORS, а отказ — просто оставленное прежним число.
    var stars = document.querySelector("[data-stars]");
    if (stars && window.fetch) {
      fetch("https://api.github.com/repos/IngvarConsulting/unica")
        .then(function (response) {
          return response.ok ? response.json() : null;
        })
        .then(function (repository) {
          if (repository && typeof repository.stargazers_count === "number") {
            stars.textContent = String(repository.stargazers_count);
          }
        })
        .catch(function () {
          /* сеть промолчала — остаётся число сборки */
        });
    }
  });
})();
