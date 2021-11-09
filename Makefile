all: index.html

index.html: index.md index.css
	pandoc --to html5 --output index.html --css index.css --standalone index.md

clean:
	rm -f index.html

.PHONY: clean
