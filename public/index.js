const init = ["Fire", "Earth", "Wind", "Water"];
const saved = JSON.parse(localStorage.getItem("data"));
const data = saved ?? init;

const reset = () => {
  localStorage.removeItem("data");
  document.getElementById("item-list").innerHTML = "";
  init.forEach((name) => {
    document.getElementById("item-list").appendChild(Item(name));
  });
};

const save = (name) => {
  data.push(name);
  console.log(data);
  localStorage.setItem("data", JSON.stringify(data));
};

const e = (tag, props = {}, children = []) => {
  const element = document.createElement(tag);
  if (props) {
    const { style, attributes, ...rest } = props;
    Object.keys(rest).forEach((key) => {
      element[key] = props[key];
    });
    Object.keys(style ?? {}).forEach((key) => {
      element.style[key] = props.style[key];
    });
    Object.keys(attributes ?? {}).forEach((key) => {
      element.setAttribute(key, props.attributes[key]);
    });
  }
  if (children) {
    children.forEach((child) => {
      element.appendChild(child);
    });
  }
  return element;
};

const getName = (el) => el.getAttribute("name");
const getItem = (name) => document.querySelector(`#item-list [name="${name}"]`);
const itemExists = (name) => getItem(name) !== null;
const isUndef = (name) => name.toLowerCase() === "undefined";

const pingItem = (name) => {
  const el = getItem(name);
  if (el !== null) {
    el.classList.add("ping");
    el.ontransitionend = () => el.classList.remove("ping");
  }
};

const stringToColor = (name) => {
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = name.charCodeAt(i) + ((hash << 5) - hash);
  }
  const hue = Math.abs(hash) % 360;
  const saturation = 60;
  const lightness = 90;

  return `hsl(${hue}, ${saturation}%, ${lightness}%)`;
};

const LogItem = (msg) => e("li", {}, [document.createTextNode(msg)]);

const journal = e("code", {
  style: {
    display: "flex",
    flexDirection: "column",
    placeContent: "end",
    listStyle: "none",
    height: "16rem",
    overflow: "hidden",
    padding: "1rem",
    position: "fixed",
    webkitMaskImage: "linear-gradient(180deg, transparent 20%, black 90%)",
    bottom: 0,
    left: 0,
  },
});

const log = (msg) => {
  journal.appendChild(LogItem(msg));
};

const Item = (name) =>
  e(
    "li",
    {
      attributes: { name },
      className: "draggable item",
      draggable: true,
      ondragenter: (event) => event.preventDefault(),
      ondragstart: (event) => {
        event.target.classList.add("dragging");
        event.dataTransfer.setData(
          "text/plain",
          event.target.attributes.name.value
        );
      },
      ondragend: (event) => event.target.classList.remove("dragging"),
      ondragover: (event) => {
        event.preventDefault();
        event.target.classList.add("incoming");
      },
      ondragleave: (event) => event.target.classList.remove("incoming"),
      ondrop: async (event) => {
        event.target.classList.remove("incoming");
        const a = getName(event.target);
        const b = event.dataTransfer.getData("text/plain");
        console.log(`${a} + ${b}`);
        setLoading(true);
        const res = await fetch("http://localhost:3000/wander", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
          },
          body: JSON.stringify({ a, b }),
        });
        setLoading(false);

        reComputeGraph();

        try {
          const { a, b, c } = await res.json();
          log(`${a} + ${b} = ${c}`);
          if (itemExists(c) || isUndef(c)) {
            pingItem(c);
            return;
          }
          save(c);
          document.getElementById("item-list").appendChild(Item(c));
        } catch (error) {
          console.error(error);
        }
      },
      style: {
        backgroundColor: stringToColor(name),
        fontSize: "1.5rem",
        padding: "0.5rem",
        boxShadow: "0 0 0.2rem 0 rgba(0, 0, 0, 0.2)",
        borderRadius: "0.25rem",
        height: "fit-content",
        maxWidth: "fit-content",
        flex: "0 1 auto",
      },
    },
    [document.createTextNode(name)]
  );

const graphRoot = e("div", { style: { height: "100%" } });

const list = e(
  "ul",
  {
    id: "item-list",
    style: {
      gap: "1rem",
      display: "flex",
      listStyle: "none",
      flexFlow: "wrap",
      alignContent: "flex-start",
      padding: "1rem",
    },
  },
  data.map(Item)
);

const trash = e(
  "button",
  {
    onclick: () => reset(),
    style: {
      backgroundColor: "lavender",
      border: "none",
      padding: "0.5rem",
      borderRadius: "0.25rem",
      position: "fixed",
      bottom: "1rem",
      right: "1rem",
    },
  },
  [document.createTextNode("Reset")]
);

const app = e(
  "div",
  {
    className: "app",
    style: {
      display: "grid",
      gridTemplateColumns: "1fr 600px",
      minHeight: "100vh",
    },
  },
  [list, graphRoot, journal, trash]
);

const setLoading = (loading) => {
  if (loading) {
    app.style.opacity = 0.5;
  } else {
    app.style.opacity = 1;
  }
};

document.body.appendChild(app);

const myGraph = ForceGraph3D()(graphRoot)
  .width(600)
  .linkAutoColorBy("target")
  .nodeLabel((d) => `<span style="color: black;">${d.name}</span>`)
  .linkWidth(1)
  .linkOpacity(0.5)
  .nodeAutoColorBy("id")
  .backgroundColor("white");

const getNodeData = async () => {
  const r = await fetch("/explore", {
    headers: {
      "Content-Type": "application/json",
    },
  });

  const da = await r.json();

  const nodes = [...new Set(da.flatMap((x) => [x.a, x.b, x.c]))].map((id) => ({
    id,
    name: id,
  }));

  const links = da.flatMap(({ a, b, c }) => [
    { source: a, target: b },
    { source: b, target: c },
  ]);

  return { nodes, links };
};

const initData = await getNodeData();

myGraph.graphData({
  nodes: initData.nodes,
  links: initData.links,
});

let seenNodes = initData.nodes;

const reComputeGraph = async () => {
  const { nodes, links } = await getNodeData();
  if (seenNodes.length === nodes.length) return;
  seenNodes = nodes;
  myGraph.graphData({
    nodes,
    links,
  });
};

reComputeGraph();
