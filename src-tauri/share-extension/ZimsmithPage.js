// Safari supplies these two fields through its supported extension API.
// No page content, cookies, or credentials are collected.
var ExtensionPreprocessingJS = {
  run: function (arguments) {
    arguments.completionFunction({
      url: document.location.href,
      title: document.title,
    });
  },
};
